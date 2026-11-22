#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""simple-bn sender with notional sensor feeds.

Sends synthetic radar data as ASTERIX CAT034 (service messages) and CAT048 (target
reports) over UDP to one declared receiver. One feed per notional sensor; each feed has
its own SAC/SIC, its own UDP source socket, its own update rate and target mix.
Every CAT048 report sets the SIM bit (I048/020): the data is simulated.

Writes a status file (default /run/simple-bn/status.json) every 5 s for the boundary
monitor: per-feed counters, per-minute history (24 h), state, last error, ack state.

Python standard library only. The receiver address is never logged or written to status.

  simple_bn_sender.py --dest HOST:PORT --label NAME [--profiles FILE] [--feeds a,b,c]

With --scene-ref LAT,LON the notional feeds share one air picture: every feed whose profile
has a `site` is a radar at that site watching the same targets, and a profile with
"kind": "fused" is a fused feed sending CAT062 system tracks correlated and filtered from all
the radars' detections (scene.py).

With --input relay the node is an egress relay instead: a sender inside the goatnet sends
each feed's ASTERIX to this host's mesh address on the feed's src_port; the relay accepts it
from the declared senders only, checks every data block, and forwards it unchanged from the
same src_port to the receiver. With --dest none it holds (counts and drops) what it receives,
so the inside path can be proven while nothing leaves.

  simple_bn_sender.py --input relay --relay-intake MESH_IP --relay-from A,B --dest HOST:PORT|none ...

With --input fabric it is the sender inside the goatnet: it subscribes to each feed's released
key (profile `key`, always under release/) and sends each sample's payload, raw ASTERIX as the
ingress adapter publishes it, to the relay's mesh address at the feed's src_port
(--relay-dest). Needs the eclipse-zenoh Python package and a goat context directory.

  simple_bn_sender.py --input fabric --goat-context DIR --relay-dest RELAY_MESH_IP ...
"""
import argparse
import json
import math
import os
import random
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

NM_M = 1852.0
FT_M = 0.3048

# ---------------------------------------------------------------- ASTERIX framing

def fspec(frns):
    n = (max(frns) - 1) // 7 + 1
    out = bytearray(n)
    for f in frns:
        i, pos = divmod(f - 1, 7)
        out[i] |= 0x80 >> pos
    for i in range(n - 1):
        out[i] |= 0x01
    return bytes(out)


def record(items):
    frns = sorted(items)
    return fspec(frns) + b"".join(items[f] for f in frns)


def block(cat, recs):
    body = b"".join(recs)
    return struct.pack(">BH", cat, 3 + len(body)) + body


def tod(t):
    return (int(round((t % 86400) * 128)) & 0xFFFFFF).to_bytes(3, "big")


def ang16(deg):
    return int(round((deg % 360.0) * 65536 / 360.0)) & 0xFFFF


def cat034(sac, sic, t, msg_type, rot_s=None, sector_deg=None, pos=None):
    items = {1: bytes([sac, sic]), 2: bytes([msg_type]), 3: tod(t)}
    if pos is not None:
        items[11] = pos                                                  # I034/120 3D position of the source
    if sector_deg is not None:
        items[4] = bytes([int((sector_deg % 360) * 256 / 360) & 0xFF])   # I034/020
    if rot_s is not None:
        items[5] = struct.pack(">H", int(round(rot_s * 128)) & 0xFFFF)   # I034/041
    return record(items)


def cat048(sac, sic, t, tgt, with_track):
    rng_nm, az = tgt.polar()
    items = {
        1: bytes([sac, sic]),                                            # I048/010
        2: tod(t),                                                       # I048/140
        3: bytes([((3 if tgt.ssr else 1) << 5) | 0x10]),                 # I048/020 TYP, SIM=1
        4: struct.pack(">HH", min(int(rng_nm * 256), 0xFFFF), ang16(az)),  # I048/040
    }
    if tgt.ssr:
        items[5] = struct.pack(">H", tgt.mode3a & 0x0FFF)                # I048/070
        items[6] = struct.pack(">H", int(tgt.alt / FT_M / 100 * 4) & 0x3FFF)  # I048/090
    if with_track:
        items[11] = struct.pack(">H", tgt.track_no & 0x0FFF)             # I048/161
        items[12] = struct.pack(">hh", int(tgt.x / NM_M * 128), int(tgt.y / NM_M * 128))  # I048/042
        items[13] = struct.pack(">HH", min(int(tgt.speed / NM_M * (1 << 14)), 0xFFFF),
                                ang16(tgt.course))                       # I048/200
        items[14] = bytes([(0x00 if tgt.confirmed else 0x80) | ((0 if tgt.ssr else 1) << 5)])  # I048/170
    if tgt.height3d is not None:
        h = int(round(tgt.height3d / FT_M / 25))                         # I048/110, 25 ft
        items[19] = struct.pack(">H", h & 0x3FFF)
    return record(items)

# ---------------------------------------------------------------- targets

class Target:
    """kind: uas | fixed-wing | rotary | projectile | clutter"""

    def __init__(self, kind, track_no, rnd, max_nm, x=None, y=None):
        self.kind, self.track_no, self.rnd = kind, track_no, rnd
        r = rnd.uniform(0.2, 0.9) * max_nm * NM_M
        b = rnd.uniform(0, 360)
        self.x = r * math.sin(math.radians(b)) if x is None else x
        self.y = r * math.cos(math.radians(b)) if y is None else y
        self.course = rnd.uniform(0, 360)
        self.age = 0
        self.ssr = False
        self.mode3a = 0
        self.alive = True
        if kind == "uas":
            self.speed, self.alt = rnd.uniform(12, 45), rnd.uniform(50, 800)
        elif kind == "rotary":
            self.speed, self.alt = rnd.uniform(30, 70), rnd.uniform(100, 1200)
            self.ssr = rnd.random() < 0.7
        elif kind == "fixed-wing":
            self.speed, self.alt = rnd.uniform(110, 240), rnd.uniform(1500, 9000)
            self.ssr = True
        elif kind == "projectile":
            self.speed = rnd.uniform(250, 700)
            self.t_flight = rnd.uniform(20, 55)
            self.apex = rnd.uniform(2500, 9000)
            self.t = 0.0
            self.alt = 0.0
        else:  # clutter
            self.speed, self.alt = 0.0, rnd.uniform(0, 300)
        if self.ssr:
            self.mode3a = int("".join(rnd.choice("01234567") for _ in range(4)), 8)

    @property
    def confirmed(self):
        return self.age >= 3

    @property
    def height3d(self):
        return self.alt

    def polar(self):
        return math.hypot(self.x, self.y) / NM_M, math.degrees(math.atan2(self.x, self.y)) % 360

    def step(self, dt, max_nm):
        self.age += 1
        if self.kind == "clutter":
            return
        if self.kind == "projectile":
            self.t += dt
            if self.t >= self.t_flight:
                self.alive = False
                return
            f = self.t / self.t_flight
            self.alt = 4 * self.apex * f * (1 - f)
        else:
            turn = {"uas": 8, "rotary": 4, "fixed-wing": 2}[self.kind]
            self.course = (self.course + self.rnd.gauss(0, turn)) % 360
        self.x += self.speed * dt * math.sin(math.radians(self.course))
        self.y += self.speed * dt * math.cos(math.radians(self.course))
        if self.kind != "projectile" and math.hypot(self.x, self.y) > max_nm * NM_M:
            self.course = (math.degrees(math.atan2(-self.x, -self.y)) + self.rnd.uniform(-30, 30)) % 360

# ---------------------------------------------------------------- notional sensor profiles
# Generic notional sensor classes, for integration testing only. Values are illustrative.
# A deployment names its own feeds with --profiles FILE (JSON: name -> profile); a file
# entry may set "base" to one of these and override any key.

PROFILES = {
    "hemispheric-cuas": {"title": "Hemispheric C-UAS radar (notional)", "sic": 1, "period": 1.0, "max_nm": 16,
                         "mix": {"uas": 6, "rotary": 1, "fixed-wing": 2}, "clutter": 2, "salvo": None},
    "counterfire": {"title": "Counterfire radar (notional)", "sic": 2, "period": 1.0, "max_nm": 32,
                    "mix": {}, "clutter": 1, "salvo": [60, 180, 2, 4]},   # every 60-180 s, 2-4 rounds
    "surveillance-3d": {"title": "3D surveillance radar (notional)", "sic": 3, "period": 2.0, "max_nm": 12,
                        "mix": {"uas": 4, "rotary": 2, "fixed-wing": 1}, "clutter": 2, "salvo": None},
}


def load_profiles(path):
    if not path or path == "none":
        return dict(PROFILES)
    with open(path) as f:
        extra = json.load(f)
    out = dict(PROFILES)
    for name, prof in extra.items():
        base = dict(PROFILES[prof["base"]]) if "base" in prof else {}
        base.update({k: v for k, v in prof.items() if k != "base"})
        out[name] = base
    return out


WINDOWS = {"1m": 60, "15m": 900, "1h": 3600, "24h": 86400}


# ---------------------------------------------------------------- source identity (SAC/SIC)
# Synthetic data always carries SAC 0, so a receiver can never take it for a real radar.
# Real data carries the radars' own SAC/SIC; a relayed or fabric feed declares the SAC/SIC it
# expects, refuses anything else, and reports what actually arrived so the monitor shows it.
SYNTHETIC_SAC = 0


def source_ids(data):
    """(cat, sac, sic) of the first record in each whole ASTERIX data block; sac/sic None without I010."""
    out, i = [], 0
    while i + 3 <= len(data):
        cat, ln = data[i], int.from_bytes(data[i + 1:i + 3], "big")
        if ln < 4 or i + ln > len(data):
            break
        j, end = i + 3, i + ln
        if data[j] & 0x80:
            while j < end and data[j] & 0x01:
                j += 1
            j += 1
            out.append((cat, data[j], data[j + 1]) if j + 2 <= end else (cat, None, None))
        else:
            out.append((cat, None, None))
        i += ln
    return out


def identity_doc(data_class, sac, sic, observed):
    """What the feed declares, what was seen, and whether they agree."""
    seen = [{"sac": k[0], "sic": k[1], "count": v["count"], "last": v["last"], "categories": sorted(v["cats"])}
            for k, v in sorted(observed.items(), key=lambda kv: -kv[1]["count"])]
    problems = []
    if data_class == "synthetic" and sac != SYNTHETIC_SAC:
        problems.append(f"synthetic data must carry SAC {SYNTHETIC_SAC}, declared {sac}")
    for o in seen:
        if (sac is not None and o["sac"] != sac) or (sic is not None and o["sic"] != sic):
            problems.append(f"saw SAC {o['sac']} / SIC {o['sic']}, expected SAC {sac if sac is not None else 'any'} / SIC {sic if sic is not None else 'any'}")
    check = "mismatch" if problems else ("unseen" if not seen else "ok")
    return {"data_class": data_class, "declared": {"sac": sac, "sic": sic}, "observed": seen,
            "check": check, "problems": problems}


class Feed:
    data_class = "synthetic"

    def __init__(self, name, sac, dest, rnd, profiles, transport="udp"):
        p = profiles[name]
        self.name, self.p, self.sac, self.sic, self.rnd = name, p, sac, p["sic"], rnd
        self.dest, self.transport = dest, transport
        self.sock, self.src_port, self.backoff, self.next_retry = None, None, 1.0, 0.0
        # A fixed source port (profile `src_port`) lets the receiver's firewall pin each feed;
        # without one the kernel picks a port, which changes on every restart.
        self.fixed_src_port = p.get("src_port")
        if transport == "udp":
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            if self.fixed_src_port:
                self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                self.sock.bind(("0.0.0.0", int(self.fixed_src_port)))
            self.sock.connect(dest)  # connected UDP: an ICMP port-unreachable surfaces as ECONNREFUSED
            self.src_port = self.sock.getsockname()[1]
        self.next_tn = 1
        self.targets = []
        for kind, n in p["mix"].items():
            for _ in range(n):
                self.targets.append(self._new(kind))
        self.next_salvo = time.time() + rnd.uniform(5, 20) if p["salvo"] else None
        self.lock = threading.Lock()
        self.total = {"datagrams": 0, "bytes": 0, "cat034": 0, "detections": 0, "tracks": 0,
                      "send_errors": 0, "refused": 0, "dropped": 0}
        self.by_kind = {}
        self.minutes = {}   # epoch-minute -> counts
        self.state = "starting"
        self.last_error = None
        self.last_send = None
        self.last_refused = None

    def _new(self, kind, **kw):
        t = Target(kind, self.next_tn, self.rnd, self.p["max_nm"], **kw)
        self.next_tn = self.next_tn % 4095 + 1
        return t

    def _observe(self, data):
        """Record the SAC/SIC of each block (call with self.lock held)."""
        obs = self.__dict__.setdefault("observed", {})
        now = time.time()
        for cat, sac, sic in source_ids(data):
            o = obs.setdefault((sac, sic), {"count": 0, "last": None, "cats": set()})
            o["count"] += 1
            o["last"] = now
            o["cats"].add(cat)

    def _count(self, key, n=1):
        m = int(time.time() // 60)
        b = self.minutes.setdefault(m, {})
        b[key] = b.get(key, 0) + n
        self.total[key] = self.total.get(key, 0) + n

    def _tcp_connect(self):
        """One TCP connection per feed, so each feed keeps its own source port. Returns True when connected."""
        if self.sock is not None:
            return True
        if time.time() < self.next_retry:
            return False
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(5)
        try:
            if self.fixed_src_port:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                s.bind(("0.0.0.0", int(self.fixed_src_port)))
            s.connect(self.dest)
        except OSError as e:
            s.close()
            with self.lock:
                self.state, self.last_error = "reconnecting", f"connect: {e}"
                if isinstance(e, ConnectionRefusedError):
                    self._count("refused")
                    self.last_refused = time.time()
            self.next_retry = time.time() + self.backoff
            self.backoff = min(self.backoff * 2, 30.0)
            return False
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock, self.src_port, self.backoff = s, s.getsockname()[1], 1.0
        with self.lock:
            self.last_refused = None   # connected: the receiver is no longer refusing
        return True

    def send(self, data, kind_counts=None):
        if self.data_class == "synthetic":
            with self.lock:
                self._observe(data)
        if self.transport == "tcp":
            if not self._tcp_connect():
                with self.lock:
                    self._count("dropped")
                return
            try:
                self.sock.sendall(data)   # ASTERIX blocks carry their own length, so the stream needs no framing
            except OSError as e:
                self.sock.close()
                self.sock = None
                with self.lock:
                    self._count("send_errors")
                    self._count("dropped")
                    self.state, self.last_error = "reconnecting", f"send: {e}"
                return
            with self.lock:
                self._count("datagrams")
                self._count("bytes", len(data))
                self.last_send = time.time()
                self.state = "sending"
            return
        try:
            self.sock.send(data)
            with self.lock:
                self._count("datagrams")
                self._count("bytes", len(data))
                self.last_send = time.time()
                self.state = "sending"
        except ConnectionRefusedError:
            with self.lock:
                self._count("refused")
                self.last_refused = time.time()
                self.state = "receiver-refusing"
                self.last_error = "ICMP port unreachable from receiver"
        except OSError as e:
            with self.lock:
                self._count("send_errors")
                self.state = "send-error"
                self.last_error = str(e)

    def scan(self):
        p, t0 = self.p, time.time()
        self.send(block(34, [cat034(self.sac, self.sic, t0, 1, rot_s=p["period"])]))
        with self.lock:
            self._count("cat034")
        if p["salvo"] and t0 >= self.next_salvo:
            lo, hi, nmin, nmax = p["salvo"]
            az = self.rnd.uniform(0, 360)
            r0 = self.rnd.uniform(0.5, 0.95) * p["max_nm"] * NM_M
            ox, oy = r0 * math.sin(math.radians(az)), r0 * math.cos(math.radians(az))
            for _ in range(self.rnd.randint(nmin, nmax)):
                pr = self._new("projectile", x=ox + self.rnd.gauss(0, 150), y=oy + self.rnd.gauss(0, 150))
                pr.course = (az + 180 + self.rnd.gauss(0, 10)) % 360   # inbound
                self.targets.append(pr)
            self.next_salvo = t0 + self.rnd.uniform(lo, hi)
        recs, kinds = [], {}
        for tg in self.targets:
            recs.append(cat048(self.sac, self.sic, time.time(), tg, with_track=True))
            kinds[tg.kind] = kinds.get(tg.kind, 0) + 1
        n_clutter = self.rnd.randint(0, p["clutter"])
        for _ in range(n_clutter):
            recs.append(cat048(self.sac, self.sic, time.time(), self._new("clutter"), with_track=False))
        # keep datagrams well under a typical MTU
        for i in range(0, len(recs), 20):
            self.send(block(48, recs[i:i + 20]))
        with self.lock:
            self._count("tracks", len(self.targets))
            self._count("detections", len(self.targets) + n_clutter)
            for k, n in kinds.items():
                self.by_kind[k] = n
        for tg in self.targets:
            tg.step(p["period"], p["max_nm"])
        self.targets = [t for t in self.targets if t.alive]
        if not p["salvo"] and self.rnd.random() < 0.01 and self.targets:   # occasional churn
            old = self.targets.pop(self.rnd.randrange(len(self.targets)))
            self.targets.append(self._new(old.kind))

    def run(self):
        k = 0
        start = time.time()
        while True:
            self.scan()
            k += 1
            nxt = start + k * self.p["period"]
            time.sleep(max(0.0, nxt - time.time()))

    def status(self, now):
        with self.lock:
            cutoff = int(now // 60) - 1440
            for m in [m for m in self.minutes if m < cutoff]:
                del self.minutes[m]
            win = {}
            for w, secs in WINDOWS.items():
                lo = (now - secs) // 60
                agg = {}
                for m, b in self.minutes.items():
                    if m >= lo:
                        for kk, v in b.items():
                            agg[kk] = agg.get(kk, 0) + v
                win[w] = agg
            stale = self.last_send is None or now - self.last_send > 3 * self.p["period"] + 5
            state = self.state
            if self.last_refused and now - self.last_refused < 15:
                state = "receiver-refusing"
            elif state == "receiver-refusing":
                state = "sending"
            if stale and state == "sending":
                state = "stalled"
            return {
                "feed": self.name, "title": self.p["title"], "sac": self.sac, "sic": self.sic,
                "data_class": self.data_class,
                "identity": identity_doc(self.data_class, self.sac, self.sic, self.__dict__.get("observed", {})),
                "src_port": self.src_port, "update_period_s": self.p["period"],
                "state": state,
                "last_send": self.last_send, "last_error": self.last_error,
                "transport": self.transport, "connected": self.sock is not None,
                "ack": ({"mode": "none", "note": "UDP one-way; receiver sends no acknowledgement",
                         "last_refused": self.last_refused} if self.transport == "udp" else
                        {"mode": "tcp", "note": "TCP: the receiver's kernel accepted the bytes; no application acknowledgement",
                         "last_refused": self.last_refused}),
                "live_targets_by_kind": dict(self.by_kind),
                "totals": dict(self.total), "windows": win,
                "per_minute": [[m, self.minutes.get(m, {}).get("datagrams", 0)]
                               for m in range(int(now // 60) - 59, int(now // 60) + 1)],
            }


# ---------------------------------------------------------------- one shared scene (--scene-ref)
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import scene as scn  # noqa: E402

KINDS_BY_BASE = {"hemispheric-cuas": ["uas", "rotary", "fixed-wing"], "counterfire": ["projectile"],
                 "surveillance-3d": ["uas", "rotary", "fixed-wing"]}


class SceneRadarFeed(Feed):
    """A radar at its site in the shared scene: CAT034 with its position, CAT048 of what it detects."""

    def __init__(self, name, sac, dest, rnd, profiles, scene, tracker, transport="udp"):
        super().__init__(name, sac, dest, rnd, profiles, transport=transport)
        p, site = self.p, self.p["site"]
        detects = p.get("detects") or (list(p["mix"]) + (["projectile"] if p.get("salvo") else []))
        noise = p.get("noise", {})
        self.scene, self.tracker = scene, tracker
        self.model = scn.RadarModel(name, float(site.get("east_m", 0)), float(site.get("north_m", 0)),
                                    p["max_nm"] * NM_M, detects, rnd,
                                    range_sigma_m=float(noise.get("range_m", 25.0)),
                                    az_sigma_deg=float(noise.get("az_deg", 0.3)),
                                    pd=float(p.get("pd", 0.92)), clutter=int(p.get("clutter", 2)),
                                    height_m=float(site.get("height_m", 0)))
        lat, lon = scene.frame.to_wgs84(self.model.east, self.model.north)
        self.pos = scn.i034_120(self.model.height_m, lat, lon)
        self.targets = []

    def scan(self):
        t0 = time.time()
        self.send(block(34, [cat034(self.sac, self.sic, t0, 1, rot_s=self.p["period"], pos=self.pos)]))
        with self.lock:
            self._count("cat034")
        obs, clutter, meas = self.model.observe(self.scene.snapshot())
        self.tracker.ingest(meas)
        recs = [cat048(self.sac, self.sic, time.time(), o, with_track=True) for o in obs]
        recs += [cat048(self.sac, self.sic, time.time(), c, with_track=False) for c in clutter]
        for i in range(0, len(recs), 20):
            self.send(block(48, recs[i:i + 20]))
        kinds = {}
        for o in obs:
            kinds[o.kind] = kinds.get(o.kind, 0) + 1
        with self.lock:
            self._count("tracks", len(obs))
            self._count("detections", len(obs) + len(clutter))
            self.by_kind = kinds


class FusedFeed(Feed):
    """System tracks after correlation and filtering, as ASTERIX CAT062, one update per period."""

    def __init__(self, name, sac, dest, rnd, profiles, scene, tracker, transport="udp"):
        p = dict(profiles[name])
        p.update(mix={}, salvo=None, clutter=0, max_nm=1, period=float(p.get("period", 1.0)))
        p.setdefault("title", name)
        super().__init__(name, sac, dest, rnd, {name: p}, transport=transport)
        self.scene, self.tracker = scene, tracker
        self.service_id = int(p.get("service_id", 1))

    def scan(self):
        trks = self.tracker.confirmed()
        recs = [scn.cat062_track(self.sac, self.sic, time.time(), tk, self.scene.frame, self.service_id,
                                 tod=tod, record=record) for tk in trks]
        for i in range(0, len(recs), 20):
            self.send(block(62, recs[i:i + 20]))
        kinds = {}
        for tk in trks:
            kinds[tk["kind"]] = kinds.get(tk["kind"], 0) + 1
        with self.lock:
            self._count("tracks", len(trks))
            self._count("multisensor", sum(1 for tk in trks if not tk["mono"]))
            self.by_kind = kinds
            if not recs:
                self.last_send = time.time()   # an empty picture is a valid update, not a stall


def build_scene(ref, names, profiles, rnd):
    """The shared scene, its tracker and the feeds: radars (profiles with `site`), fused (kind fused)."""
    lat, lon = (float(v) for v in ref.split(","))
    frame = scn.Frame(lat, lon)
    radars = [n for n in names if "site" in profiles[n]]
    if not radars:
        raise SystemExit("simple-bn: refusing to start: --scene-ref needs feeds whose profiles have a site")
    population, salvo, salvo_r, radius = {}, None, None, 0.0
    for n in radars:
        p = profiles[n]
        for k, c in p["mix"].items():
            population[k] = max(population.get(k, 0), c)
        reach = p["max_nm"] * NM_M + math.hypot(p["site"].get("east_m", 0), p["site"].get("north_m", 0))
        radius = max(radius, 0.85 * reach)
        if p.get("salvo"):
            salvo, salvo_r = p["salvo"], 0.8 * p["max_nm"] * NM_M
    return scn.Scene(frame, rnd, Target, population, radius, salvo=salvo, salvo_origin_m=salvo_r), scn.Tracker()


# ---------------------------------------------------------------- egress relay (--input relay)
# The boundary host relays what a sender inside the goatnet produces (real sensor data, read
# from the fabric) to the receiver: it adds nothing, changes nothing, and decides only whether
# a datagram may leave. A datagram leaves only if it came from a declared sender's address and
# every ASTERIX data block in it is whole, of a declared category, and (when the profile names
# them) from the declared SAC/SIC.

def asterix_check(data, cats, sac=None, sic=None):
    """None if the datagram may be relayed, else a short reason. Never raises."""
    if not data:
        return "empty"
    i = 0
    while i < len(data):
        if len(data) - i < 3:
            return "truncated"
        cat, ln = data[i], int.from_bytes(data[i + 1:i + 3], "big")
        if ln < 4 or i + ln > len(data):
            return "length"
        if cat not in cats:
            return "category"
        if sac is not None or sic is not None:
            j, end = i + 3, i + ln
            if not data[j] & 0x80:                 # FRN 1 = I010 Data Source Identifier (CAT034 and CAT048)
                return "no-source-id"
            while j < end and data[j] & 0x01:      # skip the rest of the FSPEC
                j += 1
            j += 1
            if j + 2 > end:
                return "truncated"
            if (sac is not None and data[j] != sac) or (sic is not None and data[j + 1] != sic):
                return "source-id"
        i += ln
    return None


class RelayFeed(Feed):
    """One relayed feed: intake on (mesh address, src_port), output from (any, src_port) to the receiver."""

    def __init__(self, name, dest, profiles, intake_addr, allowed, transport="udp"):
        p = dict(profiles[name])
        if not p.get("src_port"):
            raise ValueError(f"relay feed {name!r} needs a src_port in its profile")
        p.setdefault("period", 1.0)
        p.setdefault("title", name)
        self.name, self.p, self.dest, self.transport = name, p, dest, transport
        self.sac, self.sic = p.get("sac"), p.get("sic")
        self.data_class = p.get("data_class", "real")
        self.cats = set(p.get("cats", [34, 48]))
        self.allowed = set(allowed)
        self.fixed_src_port = int(p["src_port"])
        self.src_port = self.fixed_src_port
        self.sock, self.backoff, self.next_retry = None, 1.0, 0.0
        self.isock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.isock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.isock.bind((intake_addr, self.fixed_src_port))
        if dest is not None and transport == "udp":
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            self.sock.bind(("0.0.0.0", self.fixed_src_port))
            self.sock.connect(dest)
        self.lock = threading.Lock()
        self.total = {"received": 0, "rejected_source": 0, "rejected_format": 0, "held": 0,
                      "datagrams": 0, "bytes": 0, "send_errors": 0, "refused": 0, "dropped": 0}
        self.by_kind, self.minutes = {}, {}
        self.state, self.last_error, self.last_send, self.last_refused, self.last_in = "waiting-for-input", None, None, None, None

    def run(self):
        while True:
            try:
                data, addr = self.isock.recvfrom(65535)
            except OSError:
                time.sleep(0.5)
                continue
            with self.lock:
                self._count("received")
            if addr[0] not in self.allowed:
                with self.lock:
                    self._count("rejected_source")
                continue
            why = asterix_check(data, self.cats, self.sac, self.sic)
            with self.lock:
                self._observe(data)
                self.last_in = time.time()
                if why:
                    self._count("rejected_format")
                    self.last_error = f"rejected from the sender: {why}"
            if why:
                continue
            if self.dest is None:
                with self.lock:
                    self._count("held")
                continue
            self.send(data)

    def status(self, now):
        doc = super().status(now)
        idle = self.last_in is None or now - self.last_in > 30
        if idle:
            doc["state"] = "waiting-for-input"
        elif self.dest is None:
            doc["state"] = "holding"
        doc["sac"] = self.sac
        doc["update_period_s"] = None
        doc["connected"] = self.sock is not None
        doc["relay"] = {"intake_port": self.fixed_src_port, "senders": len(self.allowed),
                        "categories": sorted(self.cats), "last_input": self.last_in,
                        "forwarding": self.dest is not None}
        return doc


# ---------------------------------------------------------------- fabric input (--input fabric)
# The sender inside the goatnet. Only released keys leave (ADR 1208): every feed's key is under
# release/, where a producer with a release grant has deliberately put it. Each sample's payload
# is raw ASTERIX (the ingress adapter's `<slot>/stream/asterix/<cat>/raw/v1` frames, carried into
# the release lane unchanged). It is checked like the relay checks it and sent on, one datagram
# per sample, to the relay's mesh address at the feed's src_port.

def zenoh_config(context=None, config_file=None):
    """A zenoh.Config: an explicit JSON5 file (tests, labs), or a goat context directory (mTLS client)."""
    import zenoh
    if config_file:
        return zenoh.Config.from_file(config_file)
    import tomllib
    with open(os.path.join(context, "profile.toml"), "rb") as fh:
        prof = tomllib.load(fh)
    endpoint = prof.get("router_endpoint") or prof.get("router") or ""
    if not endpoint:
        raise SystemExit("simple-bn: refusing to start: the goat context has no router_endpoint")
    conf = zenoh.Config()
    conf.insert_json5("mode", '"client"')
    conf.insert_json5("connect/endpoints", json.dumps([endpoint]))
    conf.insert_json5("transport/link/tls", json.dumps({
        "root_ca_certificate": os.path.join(context, "ca-roots.pem"),
        "connect_certificate": os.path.join(context, "mtls.cert.pem"),
        "connect_private_key": os.path.join(context, "mtls.key.pem"),
        "enable_mtls": True,
        "verify_name_on_connect": True,
    }))
    return conf


class FabricFeed(Feed):
    """One feed read from a released key and sent on, payload unchanged."""

    def __init__(self, name, dest, profiles, transport="udp"):
        p = dict(profiles[name])
        key = p.get("key", "")
        if not key.startswith("release/"):
            raise ValueError(f"fabric feed {name!r}: key {key!r} is not under release/ (only released data leaves)")
        p.setdefault("period", 1.0)
        p.setdefault("title", name)
        self.name, self.p, self.key, self.dest, self.transport = name, p, key, dest, transport
        self.sac, self.sic = p.get("sac"), p.get("sic")
        self.data_class = p.get("data_class", "real")
        self.cats = set(p.get("cats", [34, 48]))
        self.fixed_src_port = p.get("src_port")
        self.sock, self.backoff, self.next_retry = None, 1.0, 0.0
        if transport == "udp":
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            self.sock.connect(dest)
        self.src_port = self.sock.getsockname()[1] if self.sock else None
        self.lock = threading.Lock()
        self.total = {"received": 0, "rejected_format": 0, "datagrams": 0, "bytes": 0,
                      "send_errors": 0, "refused": 0, "dropped": 0}
        self.by_kind, self.minutes = {}, {}
        self.state, self.last_error, self.last_send, self.last_refused, self.last_in = "waiting-for-input", None, None, None, None

    def on_payload(self, data):
        with self.lock:
            self._count("received")
            self._observe(data)
            self.last_in = time.time()
        why = asterix_check(data, self.cats, self.sac, self.sic)
        if why:
            with self.lock:
                self._count("rejected_format")
                self.last_error = f"rejected from the fabric: {why}"
            return
        self.send(data)

    def on_sample(self, sample):
        try:
            self.on_payload(bytes(sample.payload.to_bytes() if hasattr(sample.payload, "to_bytes") else sample.payload))
        except Exception as e:   # a subscriber callback must never raise into zenoh
            with self.lock:
                self._count("dropped")
                self.last_error = f"sample: {e}"

    def run(self):
        while True:
            time.sleep(3600)

    def status(self, now):
        doc = super().status(now)
        if self.last_in is None or now - self.last_in > 30:
            doc["state"] = "waiting-for-input"
        doc["update_period_s"] = None
        doc["fabric"] = {"key": self.key, "categories": sorted(self.cats), "last_input": self.last_in}
        return doc


def wg_route_check(dest_ip, interface):
    """None if the kernel routes dest_ip through the WireGuard interface, else a reason."""
    try:
        out = subprocess.run(["ip", "route", "get", dest_ip], capture_output=True, text=True, timeout=5).stdout
    except (OSError, subprocess.SubprocessError) as e:
        return f"cannot read the route to the receiver: {e}"
    if f" dev {interface} " not in f" {out} ":
        return f"the receiver is not routed through {interface}; bring the tunnel up first"
    return None


# ---------------------------------------------------------------- receiver feedback (goat.bn-feedback/v1)
# The receiver tells us what it sees, so integration questions are answered by the systems
# themselves rather than out of band. One JSON object per UDP datagram, at most 4 KiB:
#   {"type": "hello",   "system": "...", "contact": "...", "version": "..."}
#   {"type": "ack",     "sic": 42, "received": 118, "window_s": 60, "last_tod": 51234.5}
#   {"type": "status",  "receiving": true, "decoding": true, "sics": [42, 53], "positions_set": true, "note": "..."}
#   {"type": "request", "sac": 7, "transport": "tcp", "wg_public_key": "...", "wg_endpoint": "host:port", "note": "..."}
# Accepted from the receiver's address only (plus --feedback-allow addresses), either on the
# dedicated --feedback-port or as a reply to a feed's own source port. Anything else is
# counted as rejected and dropped. Strings are cut to 200 characters; unknown keys are dropped.

FEEDBACK_FIELDS = {
    "hello": {"system": str, "contact": str, "version": str},
    "ack": {"sic": int, "received": int, "window_s": int, "last_tod": (int, float)},
    "status": {"receiving": bool, "decoding": bool, "sics": list, "positions_set": bool, "note": str},
    "request": {"sac": int, "sic": int, "transport": str, "wg_public_key": str, "wg_endpoint": str, "note": str},
}


def parse_feedback(data):
    """A clean feedback message, or None. Never raises."""
    if len(data) > 4096:
        return None
    try:
        m = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        return None
    if not isinstance(m, dict) or m.get("type") not in FEEDBACK_FIELDS:
        return None
    out = {"type": m["type"]}
    for k, typ in FEEDBACK_FIELDS[m["type"]].items():
        v = m.get(k)
        if v is None or isinstance(v, bool) and typ is not bool:
            continue
        if isinstance(v, typ):
            if isinstance(v, str):
                v = v[:200]
            if isinstance(v, list):
                v = [x for x in v if isinstance(x, int) and not isinstance(x, bool)][:64]
            out[k] = v
    if out["type"] == "ack" and "sic" not in out:
        return None
    return out


class Feedback:
    def __init__(self, allowed):
        self.allowed = set(allowed)
        self.lock = threading.Lock()
        self.last = {}          # type -> {"at": t, "from": "ip:port", "msg": {...}}
        self.acks = {}          # sic -> {"at": t, **ack}
        self.counts = {"accepted": 0, "rejected": 0}
        self.last_heard = None

    def take(self, data, addr, via):
        msg = parse_feedback(data) if addr[0] in self.allowed else None
        with self.lock:
            if msg is None:
                self.counts["rejected"] += 1
                return None
            now = time.time()
            self.counts["accepted"] += 1
            self.last_heard = now
            self.last[msg["type"]] = {"at": now, "via": via, "msg": msg}
            if msg["type"] == "ack":
                self.acks[msg["sic"]] = {"at": now, **msg}
        return msg

    def listen(self, sock, via):
        while True:
            try:
                data, addr = sock.recvfrom(4097)
            except OSError:
                time.sleep(1)
                continue
            self.take(data, addr, via)

    def status(self):
        with self.lock:
            return {"schema": "goat.bn-feedback/v1", "last_heard": self.last_heard, "counts": dict(self.counts),
                    "hello": self.last.get("hello"), "status": self.last.get("status"),
                    "request": self.last.get("request"),
                    "acks": {str(k): v for k, v in self.acks.items()}}


ENCRYPTION = ("none", "wireguard", "receiver-transport")
INPUTS = ("notional", "fabric", "relay")


def guard(input_kind, encryption, holding=False, mesh_hop=False):
    """Stage 0 (plain, encryption none) carries synthetic data only (ADR 1253). Returns an error string or None.

    holding: a relay with no receiver (--dest none), which sends nothing, so needs no encryption.
    mesh_hop: a fabric sender sending to the egress relay over the goatnet mesh (--relay-dest); the
    mesh encrypts that hop, and the relay enforces the receiver-side encryption itself."""
    if input_kind not in INPUTS:
        return f"unknown input {input_kind!r}"
    if encryption not in ENCRYPTION:
        return f"unknown encryption {encryption!r}"
    if holding and input_kind != "relay":
        return "--dest none is for a relay that holds; a sender needs a receiver"
    if mesh_hop and input_kind != "fabric":
        return "--relay-dest is for the fabric sender inside the goatnet"
    if encryption == "none" and input_kind != "notional" and not (holding or mesh_hop):
        return "plain sending (encryption none) carries notional data only"
    return None


def write_status(path, doc):
    d = os.path.dirname(path)
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".status.")
    with os.fdopen(fd, "w") as f:
        json.dump(doc, f)
    os.chmod(tmp, 0o600)   # bn-monitor runs as the same user (User=simple-bn), so owner-read is enough
    os.replace(tmp, path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dest", default="", help="HOST:PORT (never logged), or none for a relay that holds")
    ap.add_argument("--label", required=True, help="receiver name shown in logs and the monitor")
    ap.add_argument("--node", default=socket.gethostname(), help="boundary node name")
    ap.add_argument("--feeds", default="hemispheric-cuas,counterfire,surveillance-3d")
    ap.add_argument("--profiles", default=None, help="JSON file of named feed profiles")
    ap.add_argument("--sac", type=int, default=0)
    ap.add_argument("--status", default="/run/simple-bn/status.json")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--duration", type=float, default=0, help="seconds to run, 0 = forever")
    ap.add_argument("--transport", choices=["udp", "tcp"], default="udp")
    ap.add_argument("--input", default="notional", help="notional | relay | fabric")
    ap.add_argument("--scene-ref", default="none", help="notional: LAT,LON of the system reference point; feeds share one scene (scene.py)")
    ap.add_argument("--relay-dest", default="none", help="fabric: the egress relay's mesh address; each feed goes to it at its src_port")
    ap.add_argument("--goat-context", default="none", help="fabric: goat context directory (profile.toml, mtls.cert.pem, mtls.key.pem, ca-roots.pem)")
    ap.add_argument("--zenoh-config", default="none", help="fabric: a zenoh JSON5 config file instead of a goat context (labs, tests)")
    ap.add_argument("--relay-intake", default="none", help="relay: this host's mesh address, where the inside sender sends")
    ap.add_argument("--relay-from", default="none", help="relay: the inside senders' addresses, comma-separated")
    ap.add_argument("--wg-interface", default="none", help="encryption wireguard: the tunnel the receiver must be routed through")
    ap.add_argument("--encryption", default="none", help="none | wireguard | receiver-transport (what protects the path)")
    ap.add_argument("--feedback-port", type=int, default=0, help="dedicated UDP port for receiver feedback (0 = off)")
    ap.add_argument("--feedback-allow", default="", help="extra source addresses allowed to send feedback, comma-separated")
    a = ap.parse_args()

    holding = a.dest == "none"
    mesh_hop = a.relay_dest != "none"
    err = guard(a.input, a.encryption, holding, mesh_hop)
    if not err and not a.dest and not mesh_hop:
        err = "--dest is required (or --relay-dest for the fabric sender)"
    if not err and a.input == "fabric" and a.goat_context == "none" and a.zenoh_config == "none":
        err = "the fabric input needs --goat-context (or --zenoh-config)"
    if err:
        sys.exit(f"simple-bn: refusing to start: {err}")
    dest = None
    if mesh_hop:
        a.dest = ""
    elif not holding:
        host, port = a.dest.rsplit(":", 1)
        dest = (socket.gethostbyname(host), int(port))
    if dest and a.encryption == "wireguard":
        err = (wg_route_check(dest[0], a.wg_interface) if a.wg_interface != "none"
               else "encryption wireguard needs --wg-interface")
        if err:
            sys.exit(f"simple-bn: refusing to start: {err}")
    rnd = random.Random(a.seed)
    profiles = load_profiles(a.profiles)
    names = [n.strip() for n in a.feeds.split(",") if n.strip()]
    if a.input == "relay":
        senders = [x.strip() for x in a.relay_from.split(",") if x.strip() and x.strip() != "none"]
        if a.relay_intake == "none" or (dest and not senders):
            sys.exit("simple-bn: refusing to start: a relay needs --relay-intake, and one that forwards needs at least one --relay-from address")
        feeds = [RelayFeed(n, dest, profiles, a.relay_intake, senders, transport=a.transport) for n in names]
    elif a.input == "fabric":
        relay_ip = socket.gethostbyname(a.relay_dest) if mesh_hop else None
        feeds = [FabricFeed(n, (relay_ip, int(profiles[n]["src_port"])) if mesh_hop else dest, profiles,
                            transport=a.transport) for n in names]
        import zenoh
        session = zenoh.open(zenoh_config(None if a.goat_context == "none" else a.goat_context,
                                          None if a.zenoh_config == "none" else a.zenoh_config))
        subscribers = [session.declare_subscriber(f.key, f.on_sample) for f in feeds]   # noqa: F841 (kept alive)
    elif a.scene_ref != "none":
        sc, tracker = build_scene(a.scene_ref, names, profiles, random.Random(rnd.random()))
        threading.Thread(target=sc.run, daemon=True, name="scene").start()
        feeds = []
        for n in names:
            kind = (SceneRadarFeed if "site" in profiles[n] else
                    FusedFeed if profiles[n].get("kind") == "fused" else Feed)
            args = (n, a.sac, dest, random.Random(rnd.random()), profiles)
            feeds.append(kind(*args, sc, tracker, transport=a.transport) if kind is not Feed
                         else Feed(*args, transport=a.transport))
    else:
        feeds = [Feed(n, a.sac, dest, random.Random(rnd.random()), profiles, transport=a.transport) for n in names]
    started = time.time()
    fb = Feedback(([dest[0]] if dest else []) + [x.strip() for x in a.feedback_allow.split(",") if x.strip()])
    for f in feeds:
        if f.transport == "udp" and f.sock is not None:   # replies to the feed's own source port, from the receiver's port only (connected socket)
            threading.Thread(target=fb.listen, args=(f.sock, f"reply:{f.src_port}"), daemon=True).start()
    if a.feedback_port:
        fsock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        fsock.bind(("0.0.0.0", a.feedback_port))
        threading.Thread(target=fb.listen, args=(fsock, f"port:{a.feedback_port}"), daemon=True).start()
    for f in feeds:
        threading.Thread(target=f.run, daemon=True, name=f.name).start()
    print(json.dumps({"event": "start", "node": a.node, "receiver": a.label,
                      "feeds": [{"feed": f.name, "sac": f.sac, "sic": f.sic, "src_port": f.src_port}
                                for f in feeds]}), flush=True)
    os.makedirs(os.path.dirname(a.status), exist_ok=True)
    last_log = 0
    while True:
        time.sleep(5)
        now = time.time()
        doc = {"schema": "goat.boundary-status/v1", "kind": "simple-bn", "node": a.node,
               "service": "simple-bn-sender", "input": a.input,
               "receiver": {"label": a.label, "transport": a.transport, "declared": dest is not None or mesh_hop,
                            "via_relay": mesh_hop,
                                                         "format": "ASTERIX CAT034/CAT048" + ("/CAT062" if any(isinstance(f, FusedFeed) for f in feeds) else ""),
                                                         "encryption": a.encryption},
               "started": started, "now": now, "uptime_s": round(now - started),
               "feeds": [f.status(now) for f in feeds],
               "feedback": dict(fb.status(), port=a.feedback_port or None)}
        acks = doc["feedback"]["acks"]
        for fd in doc["feeds"]:   # an application ack for this SIC in the last 2 minutes upgrades the feed's ack state
            ak = acks.get(str(fd["sic"]))
            if ak and now - ak["at"] < 120:
                sent = fd["windows"]["1m" if ak.get("window_s", 60) <= 60 else "15m"].get("datagrams", 0)
                fd["ack"] = {"mode": "app", "note": "the receiver reports what it received", "at": ak["at"],
                             "receiver_received": ak.get("received"), "window_s": ak.get("window_s"),
                             "we_sent_in_window": sent, "last_tod": ak.get("last_tod"),
                             "last_refused": fd["ack"].get("last_refused")}
        write_status(a.status, doc)
        if now - last_log >= 60:
            print(json.dumps({"event": "stats", "uptime_s": doc["uptime_s"],
                              "feeds": {f["feed"]: {"state": f["state"], **f["totals"]} for f in doc["feeds"]}}),
                  flush=True)
            last_log = now
        if a.duration and now - started >= a.duration:
            return


if __name__ == "__main__":
    main()
