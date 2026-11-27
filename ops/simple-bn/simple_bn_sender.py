#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""simple-bn sender with notional sensor feeds.

Sends synthetic radar data as ASTERIX CAT034 (service messages) and CAT048 (target
reports) over UDP to one declared receiver. One feed per notional sensor; each feed has
its own SAC/SIC, its own UDP source socket, its own update rate and target mix.
Every CAT048 report sets the SIM bit (I048/020): the data is simulated.

Writes a status file (default /run/simple-bn/status.json) every 5 s for the boundary
monitor: per-feed counters, per-minute history (24 h), state, last error, ack state.

With --sim-status FILE each feed's status carries its out-of-band sim/real declaration
(sim_status.py): for a source that can run a simulation without marking it, the operator says what it
is doing. The declaration is shown, never applied to what is sent. With --sim-status-url URL the sender
also reads declarations from a feeds service (GET, goat.sim-status/v1, keyed by that service's feed id;
a profile names it with "sim_status_from"). For each feed the newest declaration of the two wins.

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


def cat048(sac, sic, t, tgt, with_track, sim=True):
    rng_nm, az = tgt.polar()
    items = {
        1: bytes([sac, sic]),                                            # I048/010
        2: tod(t),                                                       # I048/140
        3: bytes([((3 if tgt.ssr else 1) << 5) | (0x10 if sim else 0)]),  # I048/020 TYP, SIM
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

# ---------------------------------------------------------------- reshape: real CAT062 into a receiver's shapes
# A receiver set up for one shape of data (say CAT048 plots and CAT034 status from each radar, CAT062
# only from one tracker) may take another shape in without displaying it. A relayed feed can declare
# `"reshape"` in its profile to re-encode what arrives into the shape the receiver already displays:
#   {"to": "radar",  "position_env": "<ENV>"}   CAT062 tracks -> CAT048 target reports, polar from the
#        radar's position, plus a CAT034 north marker carrying that position every `status_period_s`
#   {"to": "system", "reference_env": "<ENV>"}  CAT062 kept, with I062/015 service identification and
#        I062/100 Cartesian position (from the reference point) added where missing
# Either way each record leaves with the feed's declared SAC/SIC (`expect_sac`/`expect_sic`) and the
# SIM bit clear. Positions are "lat,lon[,height_m]" in an environment variable the deployment sets
# from the secret store, never in a profile or in code.
#
# Height datum (radar reshape). I048/110 is a height above mean sea level ("The height shall use mean
# sea level as the zero reference level", CAT048 edition 1.32). I062/130 is a height above the WGS-84
# ellipsoid ("Vertical distance between the target and the projection of its position on the earth's
# ellipsoid, as defined by WGS84", CAT062 edition 1.21). The two differ by the geoid undulation N at
# the place: mean sea level height = ellipsoid height - N. A radar reshape names
# `"geoid_undulation_env": "<ENV>"`, an environment variable holding N in metres for the feed's
# area, and N is subtracted from every I062/130 height before it is written to I048/110. With no
# undulation declared, a height from I062/130 is NOT written (the report leaves without I048/110
# and `heights_withheld` counts it on the status): a wrong datum reads as a valid height tens of
# metres off, and nothing downstream can tell. A height from I062/136, a flight level, is written
# as before: it is not an ellipsoid height. Like a position, N is never in a profile and never shown.
CAT062_FIXED = {1: 2, 3: 1, 4: 3, 5: 8, 6: 6, 7: 4, 8: 2, 9: 2, 10: 7, 12: 2, 15: 1, 17: 2, 18: 2, 19: 2,
                20: 2, 23: 1, 25: 2}
CAT062_COMPOUND = {   # FRN: octets of each subfield (None: a repetition count, then that many fixed units)
    11: [3, 6, 2, 2, 2, 2, 2, 1, (15,), 2, 2, 7, 2, 2, 2, 2, 2, 2, 1, 8, 1, 6, 2, 1, (8,), 2, 2, 2],   # I062/380
    14: [1, 1, 1, 1, 2, 1, 1, 1, 1, 1],                                                              # I062/290
    16: [1] * 31,                                                                                    # I062/295
    21: [2, 7, 4, 1, 4, 1, 4, 4, 3, 2, 2, (4,), 6, 1, 7, 7, 2, 7],                                   # I062/390
    24: [1, 4, 6, 2, 2, 1, 1],                                                                       # I062/110
    27: [4, 2, 4, 1, 1, 2, 2, 1],                                                                    # I062/500
    28: [2, 4, 2, 2, 2, 1],                                                                          # I062/340
}
CAT062_EXTENDED = {13: 1, 22: 1, 26: 3}   # FRN: octets per extent (FX in the last bit of each extent)


def _fspec_at(b, i):
    bits = []
    while True:
        x = b[i]
        i += 1
        bits += [bool(x & (0x80 >> k)) for k in range(7)]
        if not x & 1:
            return bits, i


def cat062_records(data):
    """The records of every CAT062 data block in a datagram, each as {FRN: item octets}."""
    out, i = [], 0
    while i + 3 <= len(data):
        cat, ln = data[i], int.from_bytes(data[i + 1:i + 3], "big")
        if cat != 62 or ln < 4 or i + ln > len(data):
            raise ValueError("not a whole CAT062 data block")
        j, end = i + 3, i + ln
        while j < end:
            bits, j = _fspec_at(data, j)
            items = {}
            for k, on in enumerate(bits):
                if not on:
                    continue
                f, s = k + 1, j
                if f in CAT062_FIXED:
                    j += CAT062_FIXED[f]
                elif f in CAT062_EXTENDED:
                    u = CAT062_EXTENDED[f]
                    while True:
                        j += u
                        if not data[j - 1] & 1:
                            break
                elif f in CAT062_COMPOUND:
                    sub, j = _fspec_at(data, j)
                    for n, son in enumerate(sub):
                        if son:
                            size = CAT062_COMPOUND[f][n] if n < len(CAT062_COMPOUND[f]) else 1
                            j += (1 + data[j] * size[0]) if isinstance(size, tuple) else size
                elif f in (34, 35):
                    j += data[j]
                else:
                    raise ValueError(f"CAT062 FRN {f} is not known")
                items[f] = data[s:j]
            if j > end:
                raise ValueError("CAT062 record runs past its block")
            out.append(items)
        i = end
    return out


def position_from_env(name):
    """(lat, lon, height_m) from an environment variable holding "lat,lon[,height_m]"."""
    raw = os.environ.get(name or "", "").strip()
    try:
        parts = [float(x) for x in raw.split(",")]
        lat, lon, h = parts[0], parts[1], (parts[2] if len(parts) > 2 else 0.0)
    except (ValueError, IndexError):
        raise ValueError(f"environment variable {name!r} does not hold lat,lon[,height_m]") from None
    if not (-90 <= lat <= 90 and -180 <= lon <= 180):
        raise ValueError(f"environment variable {name!r} holds a position off the earth")
    return lat, lon, h


def undulation_from_env(name):
    """The geoid undulation in metres from an environment variable."""
    raw = os.environ.get(name or "", "").strip()
    try:
        n = float(raw)
    except ValueError:
        raise ValueError(f"environment variable {name!r} does not hold a geoid undulation in metres") from None
    if not -110 <= n <= 90:   # EGM96 undulations lie between about -107 m and +86 m
        raise ValueError(f"environment variable {name!r} holds no geoid undulation of the earth")
    return n


class SeenTrack:
    """One CAT062 track seen from a site: the attributes cat048() reads from a Target."""

    def __init__(self, items, site):
        lat0, lon0 = site[0], site[1]
        lat = int.from_bytes(items[5][:4], "big", signed=True) * 180 / (1 << 25)
        lon = int.from_bytes(items[5][4:], "big", signed=True) * 180 / (1 << 25)
        self.y = math.radians(lat - lat0) * scn.EARTH_R
        self.x = math.radians(lon - lon0) * scn.EARTH_R * math.cos(math.radians(lat0))
        vx = vy = 0.0
        if 7 in items:
            vx = int.from_bytes(items[7][:2], "big", signed=True) / 4
            vy = int.from_bytes(items[7][2:], "big", signed=True) / 4
        self.speed, self.course = math.hypot(vx, vy), math.degrees(math.atan2(vx, vy)) % 360
        self.track_no = int.from_bytes(items[12], "big") if 12 in items else 0
        self.ssr = 9 in items
        self.mode3a = int.from_bytes(items[9], "big") & 0x0FFF if self.ssr else 0
        # height_ref says what height3d is measured from: "ellipsoid" or "flight-level".
        if 18 in items:                                              # I062/130 geometric altitude, 6.25 ft
            self.height3d = int.from_bytes(items[18], "big", signed=True) * 6.25 * FT_M
            self.height_ref = "ellipsoid"
        elif 17 in items:                                            # else I062/136 measured flight level, 1/4 FL
            self.height3d = int.from_bytes(items[17], "big", signed=True) / 4 * 100 * FT_M
            self.height_ref = "flight-level"
        else:
            self.height3d = None
            self.height_ref = None
        self.alt = self.height3d or 0.0
        self.confirmed = not (13 in items and items[13][0] & 0x02)   # I062/080 CNF: 1 = tentative

    def polar(self):
        return math.hypot(self.x, self.y) / NM_M, math.degrees(math.atan2(self.x, self.y)) % 360


def tod_epoch(items, now):
    """The epoch time of a record's I062/070 time of day, on the day nearest now."""
    if 4 not in items:
        return now
    t = now - (now % 86400) + int.from_bytes(items[4], "big") / 128
    return t - 86400 if t - now > 43200 else t + 86400 if now - t > 43200 else t


class Reshape:
    """Re-encodes one relayed feed's CAT062 into the shape its profile declares."""

    def __init__(self, spec, sac, sic):
        self.to = spec.get("to")
        if self.to not in ("radar", "system"):
            raise ValueError(f"reshape to {self.to!r}: radar or system")
        if sac is None or sic is None:
            raise ValueError("reshape needs the feed's expect_sac and expect_sic: they are stamped on every record")
        self.sac, self.sic = sac, sic
        self.service_id = int(spec.get("service_id", 1))
        self.simulated = bool(spec.get("simulated", False))   # SIM bit: set for a simulated source, clear for a real one
        self.status_period = float(spec.get("status_period_s", 4.0))
        # system: gather records and send the picture once per batch_s, as a tracker does (0 = as they arrive)
        self.batch_s = float(spec.get("batch_s", 0))
        self.max_datagram = int(spec.get("max_datagram", 1200))   # stay under the tunnel MTU: no IP fragments
        # a report beyond the radar's reach is not a track (radar only); 0 = no limit below the format's 256 NM
        self.max_range_nm = float(spec.get("max_range_nm", 0))
        self.dropped = 0   # records not tracks: no position, the 0,0 status record some sensors emit, or out of range
        env = spec.get("position_env") if self.to == "radar" else spec.get("reference_env")
        self.site = position_from_env(env) if (env or self.to == "radar") else None
        # radar: the geoid undulation that turns an I062/130 ellipsoid height into the mean sea level
        # height I048/110 holds. Named and unreadable refuses to start; not named withholds such heights.
        env = spec.get("geoid_undulation_env") if self.to == "radar" else None
        self.undulation = undulation_from_env(env) if env else None
        self.heights_withheld = 0

    def seen(self, r):
        """The track of one record for a radar reshape, its height above mean sea level or none."""
        tk = SeenTrack(r, self.site)
        if tk.height_ref == "ellipsoid":
            if self.undulation is None:
                tk.height3d = None
                self.heights_withheld += 1
            else:
                tk.height3d -= self.undulation
            tk.alt = tk.height3d or 0.0
        return tk

    def is_track(self, r):
        """False for a record with no position, or at 0,0 (the status record some sensors send as a track)."""
        if 5 not in r or r[5] == bytes(8):
            return False
        if self.to == "radar":
            tk = SeenTrack(r, self.site)
            rng = tk.polar()[0]
            if rng >= 255.99 or (self.max_range_nm and rng > self.max_range_nm):
                return False
        return True

    def convert(self, data, now):
        recs = cat062_records(data)
        kept = [r for r in recs if self.is_track(r)]
        self.dropped += len(recs) - len(kept)
        recs = kept
        if self.to == "radar":
            out = [cat048(self.sac, self.sic, tod_epoch(r, now), self.seen(r), True, sim=self.simulated)
                   for r in recs]
            return block(48, out) if out else None
        out = []
        for r in recs:
            r = dict(r)
            r[1] = bytes([self.sac, self.sic])
            r.setdefault(3, bytes([self.service_id & 0xFF]))
            if 6 not in r and 5 in r and self.site:
                tk = SeenTrack(r, self.site)
                r[6] = ((int(round(tk.x * 2)) & 0xFFFFFF).to_bytes(3, "big")
                        + (int(round(tk.y * 2)) & 0xFFFFFF).to_bytes(3, "big"))
            if self.simulated and 13 in r and not r[13][0] & 0x01:   # no first extent: add it, so SIM is never lost
                r[13] = bytes([r[13][0] | 0x01, 0x00])
            if 13 in r and r[13][0] & 0x01 and len(r[13]) > 1:
                st = r[13]
                sim = 0x80 if self.simulated else 0
                if all(b & 0xFE == 0 for b in st[2:]):   # extents carrying nothing: the two-octet form a tracker sends
                    r[13] = bytes([st[0], (st[1] & 0x7E) | sim])             # SIM as declared, no further extent
                else:
                    r[13] = st[:1] + bytes([(st[1] & 0x7F) | sim]) + st[2:]   # SIM as declared
            out.append(record(r))
        return block(62, out) if out else None

    def records(self, data, now):
        """The reshaped CAT062 records of one datagram, for batching (system only)."""
        out = self.convert(data, now)
        return [] if out is None else [record(r) for r in cat062_records(out)]

    @staticmethod
    def latest(recs):
        """The newest record of each track: a source faster than the picture sends a track several times."""
        out = {}
        for rec in recs:
            (r,) = cat062_records(block(62, [rec]))
            key = r.get(12) or rec   # no track number: kept as its own record
            out.pop(key, None)
            out[key] = rec
        return list(out.values())

    def blocks(self, recs):
        """CAT062 data blocks of at most max_datagram octets holding recs, in order."""
        out, cur, size = [], [], 3
        for r in recs:
            if cur and size + len(r) > self.max_datagram:
                out.append(block(62, cur))
                cur, size = [], 3
            cur.append(r)
            size += len(r)
        if cur:
            out.append(block(62, cur))
        return out

    def status(self, now):
        """The CAT034 north marker with the radar's position, for a radar reshape."""
        if self.to != "radar":
            return None
        pos = scn.i034_120(self.site[2], self.site[0], self.site[1])
        return block(34, [cat034(self.sac, self.sic, now, 1, rot_s=self.status_period, pos=pos)])


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


# --src-addr: with encryption ipsec, every source port is bound to one address whether or not the tunnel
# is up at that moment: in policy mode our public address, which the role puts on a dummy interface, so
# each datagram matches the local traffic selector (a kernel-chosen address would be the private one);
# in route mode our tunnel address, whose routing table leads only into the tunnel.
SRC_ADDR = None


def local_addr_for(dest):
    """The local address the kernel routes to dest from: a fixed source port is bound there, never on every
    interface (on a tunnel this is the tunnel address, so nothing else can reach the port)."""
    if SRC_ADDR:
        return SRC_ADDR
    if not dest:
        return "127.0.0.1"
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect((dest[0], int(dest[1]) or 9))
        return s.getsockname()[0]
    finally:
        s.close()


def udp_connect(sock, dest):
    """Connect a UDP socket to dest. Returns None, or the error text. With encryption ipsec the receiver's
    inside address is unreachable while the tunnel is down (the role's fail-closed route), so the connect
    fails until the tunnel comes up; send() tries again and counts the send as an error meanwhile."""
    try:
        sock.connect(dest)
        return None
    except OSError as e:
        return str(e)


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


def read_sim_status(path):
    """The out-of-band sim/real declarations (sim_status.py), and how the file read. Never raises."""
    if not path or path == "none":
        return {}, {"path": None, "loaded": False, "error": None}
    try:
        with open(path) as f:
            doc = json.load(f)
        feeds = doc.get("feeds") if isinstance(doc, dict) else None
        if not isinstance(feeds, dict):
            raise ValueError("not a goat.sim-status/v1 document")
        return feeds, {"path": path, "loaded": True, "error": None}
    except FileNotFoundError:
        return {}, {"path": path, "loaded": False, "error": None}
    except (OSError, ValueError) as e:
        return {}, {"path": path, "loaded": False, "error": str(e)[:200]}


def read_sim_status_url(url, timeout=2.0):
    """Declarations from a feeds service's GET endpoint (goat.sim-status/v1). Never raises; the URL is not echoed."""
    if not url or url == "none":
        return {}, {"source": None, "loaded": False, "error": None}
    import urllib.request
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            doc = json.loads(r.read(1 << 20).decode("utf-8"))
        feeds = doc.get("feeds") if isinstance(doc, dict) else None
        if not isinstance(feeds, dict):
            raise ValueError("not a goat.sim-status/v1 document")
        return feeds, {"source": "url", "loaded": True, "error": None}
    except (OSError, ValueError) as e:
        return {}, {"source": "url", "loaded": False, "error": type(e).__name__ + ": " + str(e)[:160]}


def merge_declarations(feeds, from_file, from_url):
    """Per local feed name, the newest valid declaration: the host file by feed name, the feeds service by the
    profile's sim_status_from id. Each kept declaration carries its source ("host" or "feeds-service")."""
    out = {}
    for f in feeds:
        p = getattr(f, "p", {}) or {}
        cands = [(from_file.get(f.name), "host")]
        if p.get("sim_status_from"):
            cands.append((from_url.get(str(p["sim_status_from"])), "feeds-service"))
        best = None
        for d, src in cands:
            if not isinstance(d, dict) or d.get("mode") not in ("real", "simulated", "unknown"):
                continue
            at = d.get("set_at") if isinstance(d.get("set_at"), (int, float)) else 0
            if best is None or at > best[0]:
                best = (at, dict(d, source=src))
        if best:
            out[f.name] = best[1]
    return out


def sim_bit_sent(feed):
    """The SIM bit this node puts on the feed's records: True, False, or None when records pass unchanged."""
    if isinstance(feed, (RelayFeed, FabricFeed)):
        rs = getattr(feed, "reshape", None)
        return rs.simulated if rs else None
    return True   # generated feeds are always simulated


def sim_status_doc(feed, declared):
    """Out-of-band sim/real status of one feed: what is declared, on what basis, and whether the SIM bit agrees."""
    p = getattr(feed, "p", {}) or {}
    d = declared.get(feed.name) if isinstance(declared, dict) else None
    if isinstance(d, dict) and d.get("mode") in ("real", "simulated", "unknown"):
        doc = {"declared": d["mode"], "basis": "operator", "set_at": d.get("set_at"),
               "set_by": d.get("set_by"), "note": d.get("note") or None, "source": d.get("source", "host")}
    else:
        doc = {"declared": "real" if feed.data_class == "real" else "simulated", "basis": "profile",
               "set_at": None, "set_by": None, "note": None, "source": None}
    bit = sim_bit_sent(feed)
    doc["source_can_simulate"] = bool(p.get("source_can_simulate", feed.data_class != "real"))
    doc["source_marks_simulation"] = bool(p.get("source_marks_simulation", False))
    doc["sim_bit_sent"] = bit
    doc["agrees"] = None if bit is None or doc["declared"] == "unknown" else (doc["declared"] == "simulated") == bit
    return doc


class Feed:
    data_class = "synthetic"
    connect_error = None   # set by _udp_connect: None when connected, else why not (tried again on send)

    def _udp_connect(self, dest):
        self.connect_error = udp_connect(self.sock, dest)

    def __init__(self, name, sac, dest, rnd, profiles, transport="udp", generate=True):
        if not generate:   # relayed and fabric feeds forward real data and set up their own state
            return
        p = profiles[name]
        self.name, self.p, self.sac, self.sic, self.rnd = name, p, sac, p["sic"], rnd
        if dest is not None and p.get("dest_port"):   # the receiver takes this feed on its own port
            dest = (dest[0], int(p["dest_port"]))
        self.dest, self.transport = dest, transport
        self.sock, self.src_port, self.backoff, self.next_retry = None, None, 1.0, 0.0
        # A fixed source port (profile `src_port`) lets the receiver's firewall pin each feed;
        # without one the kernel picks a port, which changes on every restart.
        self.fixed_src_port = p.get("src_port")
        if transport == "udp":
            self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            if self.fixed_src_port:
                self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                self.sock.bind((local_addr_for(dest), int(self.fixed_src_port)))
            self._udp_connect(dest)  # connected UDP: an ICMP port-unreachable surfaces as ECONNREFUSED
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

    def _observe_out(self, data):
        """Record the category and SAC/SIC of each block as it leaves for the receiver (lock held)."""
        out = self.__dict__.setdefault("sent_ids", {})
        now = time.time()
        for cat, sac, sic in source_ids(data):
            o = out.setdefault((cat, sac, sic), {"count": 0, "last": None})
            o["count"] += 1
            o["last"] = now

    def modifications(self):
        """What this node changes between what arrives (or is generated) and what the receiver gets:
        the identities and categories on each side, measured from the blocks themselves, and the changes
        the profile declares. GET /api/v1/modifications on the monitor."""
        obs = self.__dict__.get("observed", {})
        sent = self.__dict__.get("sent_ids", {})
        inp = [{"sac": k[0], "sic": k[1], "categories": sorted(v["cats"]), "count": v["count"], "last": v["last"]}
               for k, v in sorted(obs.items(), key=lambda kv: -kv[1]["count"])]
        outp = [{"category": k[0], "sac": k[1], "sic": k[2], "count": v["count"], "last": v["last"]}
                for k, v in sorted(sent.items(), key=lambda kv: -kv[1]["count"])]
        relayed = isinstance(self, (RelayFeed, FabricFeed))
        changes = []
        rs = getattr(self, "reshape", None)
        if rs:
            changes.append({"kind": "format", "from": "CAT062",
                            "to": "CAT048 plots + CAT034 status (a radar)" if rs.to == "radar" else "CAT062 system tracks",
                            "why": "reshape in the profile: the receiver's display takes this shape"})
        in_ids = {(o["sac"], o["sic"]) for o in inp}
        out_ids = {(o["sac"], o["sic"]) for o in outp}
        for a in sorted(i for i in in_ids if i not in out_ids and None not in i):
            for b in sorted(i for i in out_ids if i not in in_ids and None not in i):
                changes.append({"kind": "identity", "from": {"sac": a[0], "sic": a[1]}, "to": {"sac": b[0], "sic": b[1]},
                                "why": "the identity agreed with the receiver is stamped on every record"})
        if rs and rs.simulated:
            changes.append({"kind": "sim-bit", "to": "set", "why": "the feed is declared simulated"})
        return {"source": "relayed" if relayed else "generated on this node",
                "input": inp if relayed else [], "output": outp, "changes": changes,
                "unchanged": relayed and bool(inp) and bool(outp) and not changes}

    def identity(self):
        """Declared SAC/SIC: enforced ones (profile sac/sic), else expected ones checked and shown only
        (profile expect_sac/expect_sic), so a real feed whose source is not yet stamped right still flows."""
        sac = self.sac if self.sac is not None else self.p.get("expect_sac")
        sic = self.sic if self.sic is not None else self.p.get("expect_sic")
        doc = identity_doc(self.data_class, sac, sic, self.__dict__.get("observed", {}))
        doc["enforced"] = self.sac is not None or self.sic is not None
        return doc

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
                s.bind((local_addr_for(self.dest), int(self.fixed_src_port)))
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
                self._observe_out(data)
                self.last_send = time.time()
                self.state = "sending"
            return
        if self.connect_error and not getattr(self, "sock_dest", None):
            self._udp_connect(self.dest)
            if self.connect_error:
                with self.lock:
                    self._count("send_errors")
                    self.state = "send-error"
                    self.last_error = f"connect: {self.connect_error}"
                return
        try:
            if getattr(self, "sock_dest", None):
                self.sock.sendto(data, self.sock_dest)
            else:
                self.sock.send(data)
            with self.lock:
                self._count("datagrams")
                self._count("bytes", len(data))
                self._observe_out(data)
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
                "modifications": self.modifications(),
                "src_port": self.src_port, "dest_port": self.dest[1] if self.dest else None,
                "update_period_s": self.p["period"],
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
        # shape "real": what the relay sends for a real radar (reshape to radar): tracked plots only, the same
        # items in every plot, one plot per datagram, and CAT034 every status_period_s. Only SIM and SAC differ.
        self.real_shape = p.get("shape") == "real"
        self.status_period = float(p.get("status_period_s", 4.0))
        self.next_status = 0.0
        if self.real_shape:
            self.model.clutter = 0

    def scan(self):
        t0 = time.time()
        if not self.real_shape or t0 >= self.next_status:
            rot = self.status_period if self.real_shape else self.p["period"]
            self.send(block(34, [cat034(self.sac, self.sic, t0, 1, rot_s=rot, pos=self.pos)]))
            self.next_status = t0 + self.status_period
            with self.lock:
                self._count("cat034")
        obs, clutter, meas = self.model.observe(self.scene.snapshot())
        self.tracker.ingest(meas)
        if self.real_shape:
            for o in obs:
                o.ssr = False
        recs = [cat048(self.sac, self.sic, time.time(), o, with_track=True) for o in obs
                if not self.real_shape or o.confirmed]
        recs += [cat048(self.sac, self.sic, time.time(), c, with_track=False) for c in clutter]
        step = 1 if self.real_shape else 20
        for i in range(0, len(recs), step):
            self.send(block(48, recs[i:i + step]))
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
                                 tod=tod, record=record, shape=self.p.get("shape")) for tk in trks]
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


class BlueFeed(Feed):
    """One friendly drone that reports its own position, as CAT062: no height, track 1, SIM set."""

    def __init__(self, name, sac, dest, rnd, profiles, scene, tracker, transport="udp"):
        p = dict(profiles[name])
        p.update(mix={}, salvo=None, clutter=0, max_nm=float(p.get("max_nm", 3)))
        p.setdefault("period", 1.0)
        p.setdefault("title", name)
        super().__init__(name, sac, dest, rnd, {name: p}, transport=transport)
        self.scene, self.service_id = scene, int(p.get("service_id", 1))
        site = p.get("site_m", [0, 0])
        self.east, self.north = float(site[0]), float(site[1])
        self.drone = Target("uas", 1, rnd, p["max_nm"])

    def scan(self):
        d = self.drone
        d.step(self.p["period"], self.p["max_nm"])
        trk = {"number": 1, "x": self.east + d.x, "y": self.north + d.y, "alt": None,
               "vx": d.speed * math.sin(math.radians(d.course)), "vy": d.speed * math.cos(math.radians(d.course)),
               "mode3a": None, "mono": True}
        self.send(block(62, [scn.cat062_track(self.sac, self.sic, time.time(), trk, self.scene.frame, self.service_id,
                                              tod=tod, record=record, shape="real")]))
        with self.lock:
            self._count("tracks")
            self.by_kind = {"uas": 1}


def build_scene(ref, names, profiles, rnd):
    """The shared scene, its tracker and the feeds: radars (profiles with `site`), fused (kind fused)."""
    lat, lon = (float(v) for v in ref.split(","))
    frame = scn.Frame(lat, lon)
    radars = [n for n in names if "site" in profiles[n]]
    if not radars:
        raise SystemExit("simple-bn: refusing to start: --scene-ref needs feeds whose profiles have a site")
    areas = {}
    for n in radars:
        areas.setdefault(profiles[n].get("area"), []).append(n)
    scenes = []
    for area, members in areas.items():
        # the unnamed area is centred on the reference point; a named one on its radars
        east = north = 0.0
        if area is not None:
            east = sum(float(profiles[n]["site"].get("east_m", 0)) for n in members) / len(members)
            north = sum(float(profiles[n]["site"].get("north_m", 0)) for n in members) / len(members)
        population, salvo, salvo_r, radius = {}, None, None, 0.0
        for n in members:
            p = profiles[n]
            for k, c in p["mix"].items():
                population[k] = max(population.get(k, 0), c)
            reach = p["max_nm"] * NM_M + math.hypot(p["site"].get("east_m", 0) - east, p["site"].get("north_m", 0) - north)
            radius = max(radius, 0.85 * reach)
            if p.get("salvo"):
                salvo, salvo_r = p["salvo"], 0.8 * p["max_nm"] * NM_M
        scenes.append((east, north, scn.Scene(frame, rnd, Target, population, radius, salvo=salvo, salvo_origin_m=salvo_r)))
    sc = scenes[0][2] if list(areas) == [None] else scn.Areas(scenes)
    return sc, scn.Tracker()


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
        if dest is not None and profiles[name].get("dest_port"):   # the receiver takes this feed on its own port
            dest = (dest[0], int(profiles[name]["dest_port"]))
        super().__init__(name, None, dest, None, profiles, transport, generate=False)
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
            src = local_addr_for(dest)
            if src == intake_addr:   # intake and egress on one address (a lab): send from the intake socket itself
                self.sock, self.sock_dest = self.isock, dest
            else:
                self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                self.sock.bind((src, self.fixed_src_port))
                self._udp_connect(dest)
        spec = p.get("reshape")
        self.reshape = Reshape(spec, p.get("expect_sac", p.get("sac")), p.get("expect_sic", p.get("sic"))) if spec else None
        self.lock = threading.Lock()
        self.total = {"received": 0, "rejected_source": 0, "rejected_format": 0, "held": 0,
                      "datagrams": 0, "bytes": 0, "send_errors": 0, "refused": 0, "dropped": 0}
        self.by_kind, self.minutes = {}, {}
        self.state, self.last_error, self.last_send, self.last_refused, self.last_in = "waiting-for-input", None, None, None, None

    def _batch_loop(self):
        while True:
            time.sleep(self.reshape.batch_s)
            with self.lock:
                recs, self.pending = self.pending, []
            for b in self.reshape.blocks(self.reshape.latest(recs)):
                self.send(b)

    def _status_loop(self):
        while True:
            self.send(self.reshape.status(time.time()))
            time.sleep(self.reshape.status_period)

    def run(self):
        if self.reshape and self.reshape.to == "radar" and self.dest is not None:
            threading.Thread(target=self._status_loop, daemon=True).start()
        self.pending = []
        batching = bool(self.reshape and self.reshape.to == "system" and self.reshape.batch_s > 0)
        if batching and self.dest is not None:
            threading.Thread(target=self._batch_loop, daemon=True).start()
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
            if batching:
                try:
                    recs = self.reshape.records(data, time.time())
                except (ValueError, IndexError, KeyError) as e:
                    with self.lock:
                        self._count("rejected_format")
                        self.last_error = f"not reshaped: {e}"
                    continue
                with self.lock:
                    self.pending.extend(recs)
                continue
            if self.reshape:
                try:
                    data = self.reshape.convert(data, time.time())
                except (ValueError, IndexError, KeyError) as e:
                    with self.lock:
                        self._count("rejected_format")
                        self.last_error = f"not reshaped: {e}"
                    continue
                if data is None:
                    continue
            self.send(data)

    def status(self, now):
        doc = super().status(now)
        if self.reshape:   # the position itself is never shown
            doc["reshape"] = {"to": self.reshape.to, "sac": self.reshape.sac, "sic": self.reshape.sic,
                              "position_set": self.reshape.site is not None,
                              "simulated": self.reshape.simulated}
            if self.reshape.batch_s:
                doc["reshape"]["batch_s"] = self.reshape.batch_s
            doc["reshape"]["not_tracks_dropped"] = self.reshape.dropped
            if self.reshape.to == "radar":   # the undulation itself is never shown
                doc["reshape"]["geoid_undulation_set"] = self.reshape.undulation is not None
                doc["reshape"]["heights_withheld"] = self.reshape.heights_withheld
        idle = self.last_in is None or now - self.last_in > 30
        if idle:
            doc["state"] = "waiting-for-input"
        elif self.dest is None:
            doc["state"] = "holding"
        doc["sac"] = self.sac
        doc["identity"] = self.identity()
        doc["update_period_s"] = None
        doc["connected"] = self.sock is not None
        doc["relay"] = {"intake_port": self.fixed_src_port, "senders": len(self.allowed),
                        "categories": sorted(self.cats), "last_input": self.last_in,
                        "forwarding": self.dest is not None,
                        "dest_port": self.dest[1] if self.dest else None}
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
        super().__init__(name, None, dest, None, profiles, transport, generate=False)
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
        doc["identity"] = self.identity()
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


ENCRYPTION = ("none", "wireguard", "ipsec", "receiver-transport")
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
    ap.add_argument("--sim-status", default="none",
                    help="out-of-band sim/real declarations per feed (sim_status.py writes it), or none")
    ap.add_argument("--sim-status-url", default="none",
                    help="a feeds service's GET sim-status endpoint (read every 15 s), or none")
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
    ap.add_argument("--encryption", default="none", help="none | wireguard | ipsec | receiver-transport (what protects the path)")
    ap.add_argument("--src-addr", default="none", help="encryption ipsec: our public (policy mode) or tunnel (route mode) address; every source port binds here")
    ap.add_argument("--feedback-port", type=int, default=0, help="dedicated UDP port for receiver feedback (0 = off)")
    ap.add_argument("--feedback-allow", default="", help="extra source addresses allowed to send feedback, comma-separated")
    a = ap.parse_args()
    global SRC_ADDR
    SRC_ADDR = None if a.src_addr == "none" else a.src_addr

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
        # Simulated and real side by side (operator 2026-11-22: "we will run both the real and sim for
        # a little while and then eventually just the real"). A profile with "input": "relay" or
        # "fabric" is a real feed beside the simulated scene; real feeds never travel in plain UDP, and
        # SAC separates the classes (synthetic is always SAC 0).
        real = [n for n in names if profiles[n].get("input") in ("relay", "fabric")]
        names = [n for n in names if n not in real]
        if real and a.encryption == "none" and dest:
            sys.exit("simple-bn: refusing to start: real feeds beside the simulated ones need encryption (stage 1 or 2)")
        sc, tracker = build_scene(a.scene_ref, names, profiles, random.Random(rnd.random()))
        threading.Thread(target=sc.run, daemon=True, name="scene").start()
        feeds = []
        relay_real = [n for n in real if profiles[n]["input"] == "relay"]
        fabric_real = [n for n in real if profiles[n]["input"] == "fabric"]
        if relay_real:
            senders = [x.strip() for x in a.relay_from.split(",") if x.strip() and x.strip() != "none"]
            if a.relay_intake == "none":
                sys.exit("simple-bn: refusing to start: a relayed real feed needs --relay-intake")
            # With no declared sender yet the feed holds: it shows what arrives and forwards nothing.
            feeds += [RelayFeed(n, dest if senders else None, profiles, a.relay_intake, senders, transport=a.transport)
                      for n in relay_real]
        if fabric_real:
            if a.goat_context == "none" and a.zenoh_config == "none":
                sys.exit("simple-bn: refusing to start: a fabric real feed needs --goat-context (or --zenoh-config)")
            ffeeds = [FabricFeed(n, dest, profiles, transport=a.transport) for n in fabric_real]
            import zenoh
            session = zenoh.open(zenoh_config(None if a.goat_context == "none" else a.goat_context,
                                              None if a.zenoh_config == "none" else a.zenoh_config))
            subscribers = [session.declare_subscriber(f.key, f.on_sample) for f in ffeeds]   # noqa: F841 (kept alive)
            feeds += ffeeds
        for n in names:
            kind = (SceneRadarFeed if "site" in profiles[n] else
                    FusedFeed if profiles[n].get("kind") == "fused" else
                    BlueFeed if profiles[n].get("kind") == "blue" else Feed)
            args = (n, a.sac, dest, random.Random(rnd.random()), profiles)
            feeds.append(kind(*args, sc, tracker, transport=a.transport) if kind is not Feed
                         else Feed(*args, transport=a.transport))
    else:
        feeds = [Feed(n, a.sac, dest, random.Random(rnd.random()), profiles, transport=a.transport) for n in names]
    started = time.time()
    fb = Feedback(([dest[0]] if dest else []) + [x.strip() for x in a.feedback_allow.split(",") if x.strip()])
    for f in feeds:
        if f.transport == "udp" and f.sock is not None and f.sock is not getattr(f, "isock", None):   # replies to the feed's own source port, from the receiver's port only (connected socket)
            threading.Thread(target=fb.listen, args=(f.sock, f"reply:{f.src_port}"), daemon=True).start()
    if a.feedback_port:
        # Reports arrive on the address that routes to the receiver (the tunnel, when there is one) or on the
        # address the default route leaves from (the public address, by NAT); listen on each, not on every interface.
        for addr in sorted({local_addr_for(dest), local_addr_for(("192.0.2.1", 9))}):
            fsock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            fsock.bind((addr, a.feedback_port))
            threading.Thread(target=fb.listen, args=(fsock, f"port:{a.feedback_port}"), daemon=True).start()
    for f in feeds:
        threading.Thread(target=f.run, daemon=True, name=f.name).start()
    print(json.dumps({"event": "start", "node": a.node, "receiver": a.label,
                      "feeds": [{"feed": f.name, "sac": f.sac, "sic": f.sic, "src_port": f.src_port}
                                for f in feeds]}), flush=True)
    os.makedirs(os.path.dirname(a.status), exist_ok=True)
    last_log = 0
    url_decl, sim_url, url_at = {}, read_sim_status_url("none")[1], 0
    while True:
        time.sleep(5)
        now = time.time()
        file_decl, sim_file = read_sim_status(a.sim_status)
        if a.sim_status_url != "none" and now - url_at >= 15:
            url_at = now
            got, sim_url = read_sim_status_url(a.sim_status_url)
            if sim_url["loaded"]:
                url_decl = got          # an unreachable service keeps the last declarations it gave
        declared = merge_declarations(feeds, file_decl, url_decl)
        doc = {"schema": "goat.boundary-status/v1", "kind": "simple-bn", "node": a.node,
               "service": "simple-bn-sender", "input": a.input + ("+real" if any(f.data_class == "real" for f in feeds) and a.input == "notional" else ""),
               "receiver": {"label": a.label, "transport": a.transport, "declared": dest is not None or mesh_hop,
                            "via_relay": mesh_hop,
                                                         "format": "ASTERIX CAT034/CAT048" + ("/CAT062" if any(isinstance(f, (FusedFeed, BlueFeed)) for f in feeds) else ""),
                                                         "encryption": a.encryption},
               "started": started, "now": now, "uptime_s": round(now - started),
               "feeds": [f.status(now) for f in feeds],
               "sim_status_file": sim_file,
               "sim_status_url": sim_url,
               "feedback": dict(fb.status(), port=a.feedback_port or None)}
        for f, fd in zip(feeds, doc["feeds"]):
            fd["sim_status"] = sim_status_doc(f, declared)
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
