# SPDX-License-Identifier: Apache-2.0
"""One notional air picture seen by several radars, and the system tracks fused from them.

A Scene holds the true targets in a local east/north frame (metres) around a system reference
point (SRP, latitude and longitude). Each radar sits at its own site in that frame and reports
what it can see: CAT034 service messages carrying its position (I034/120) and CAT048 target
reports with measurement noise, a detection probability that falls with range, and clutter.
A Tracker correlates every radar's tracked detections into system tracks (gated nearest
neighbour, a constant-velocity Kalman filter per axis, M-of-N confirmation, coasting, merging) and a fused feed sends them as CAT062.

Every report is simulated: CAT048 sets SIM in I048/020 and CAT062 sets SIM in I062/080.
Python standard library only.
"""
import math
import struct
import threading
import time

NM_M = 1852.0
FT_M = 0.3048
EARTH_R = 6371000.0


# ---------------------------------------------------------------- geometry

class Frame:
    """Flat-earth local tangent frame around the SRP; accurate to metres within ~100 km."""

    def __init__(self, lat, lon):
        self.lat, self.lon = lat, lon
        self.coslat = math.cos(math.radians(lat))

    def to_wgs84(self, east, north):
        return (self.lat + math.degrees(north / EARTH_R),
                self.lon + math.degrees(east / (EARTH_R * self.coslat)))


def wgs84_25(deg):
    """I062/105 coordinate: 180/2^25 degrees, signed 32 bits."""
    return int(round(deg * (1 << 25) / 180.0))


def wgs84_23(deg):
    """I034/120 coordinate: 180/2^23 degrees, signed 24 bits."""
    return int(round(deg * (1 << 23) / 180.0)) & 0xFFFFFF


# ---------------------------------------------------------------- the true scene

class Scene:
    """True targets around the SRP. stepped by its own thread, read by every radar."""

    def __init__(self, frame, rnd, target_cls, population, radius_m, salvo=None, salvo_origin_m=None):
        self.frame, self.rnd, self.Target = frame, rnd, target_cls
        self.population, self.radius_m, self.salvo = dict(population), radius_m, salvo
        self.salvo_origin_m = salvo_origin_m or radius_m * 0.8
        self.lock = threading.Lock()
        self.next_id = 1
        self.targets = []
        for kind, n in self.population.items():
            for _ in range(n):
                self.targets.append(self._new(kind))
        self.next_salvo = time.time() + rnd.uniform(5, 20) if salvo else None

    def _new(self, kind, **kw):
        t = self.Target(kind, self.next_id, self.rnd, self.radius_m / NM_M, **kw)
        t.truth_id = self.next_id
        self.next_id = self.next_id % 65535 + 1
        return t

    def step(self, dt):
        with self.lock:
            now = time.time()
            if self.salvo and now >= self.next_salvo:
                lo, hi, nmin, nmax = self.salvo
                az = self.rnd.uniform(0, 360)
                r0 = self.rnd.uniform(0.7, 1.0) * self.salvo_origin_m
                ox, oy = r0 * math.sin(math.radians(az)), r0 * math.cos(math.radians(az))
                for _ in range(self.rnd.randint(nmin, nmax)):
                    pr = self._new("projectile", x=ox + self.rnd.gauss(0, 150), y=oy + self.rnd.gauss(0, 150))
                    pr.course = (az + 180 + self.rnd.gauss(0, 10)) % 360   # inbound
                    self.targets.append(pr)
                self.next_salvo = now + self.rnd.uniform(lo, hi)
            for t in self.targets:
                t.step(dt, self.radius_m / NM_M)
            self.targets = [t for t in self.targets if t.alive]
            have = {}
            for t in self.targets:
                have[t.kind] = have.get(t.kind, 0) + 1
            for kind, n in self.population.items():   # keep the population: replace what left
                for _ in range(n - have.get(kind, 0)):
                    self.targets.append(self._new(kind))

    def snapshot(self):
        with self.lock:
            return [(t.truth_id, t.kind, t.x, t.y, t.alt, t.speed, t.course, t.ssr, t.mode3a) for t in self.targets]

    def run(self, dt=0.5):
        while True:
            self.step(dt)
            time.sleep(dt)


# ---------------------------------------------------------------- one radar's view

class Observation:
    """What a radar reports for one detection, shaped for the CAT048 encoder (polar from the site)."""

    def __init__(self, rng_m, az_deg, east_rel, north_rel, alt, speed, course, ssr, mode3a, track_no, age, kind):
        self.rng_m, self.az, self.x, self.y = rng_m, az_deg, east_rel, north_rel
        self.alt, self.speed, self.course = alt, speed, course
        self.ssr, self.mode3a, self.track_no, self.age, self.kind = ssr, mode3a, track_no, age, kind

    def polar(self):
        return self.rng_m / NM_M, self.az

    @property
    def confirmed(self):
        return self.age >= 3

    @property
    def height3d(self):
        return self.alt


class RadarModel:
    """A radar site in the scene frame: which kinds it sees, how far, how well."""

    def __init__(self, name, east_m, north_m, max_m, detects, rnd, range_sigma_m=25.0, az_sigma_deg=0.3,
                 pd=0.92, clutter=2, height_m=0.0):
        self.name, self.east, self.north, self.max_m = name, east_m, north_m, max_m
        self.detects, self.rnd = set(detects), rnd
        self.range_sigma, self.az_sigma, self.pd, self.clutter, self.height_m = range_sigma_m, az_sigma_deg, pd, clutter, height_m
        self.local_tracks = {}   # truth id -> [local track number, age]
        self.next_tn = 1

    def _local(self, truth_id):
        e = self.local_tracks.get(truth_id)
        if e is None:
            e = self.local_tracks[truth_id] = [self.next_tn, 0]
            self.next_tn = self.next_tn % 4095 + 1
        e[1] += 1
        return e

    def observe(self, snapshot):
        """(observations with track data, clutter observations, measurements in the scene frame)."""
        obs, meas, seen = [], [], set()
        for tid, kind, x, y, alt, speed, course, ssr, m3a in snapshot:
            if kind not in self.detects:
                continue
            dx, dy = x - self.east, y - self.north
            r = math.hypot(dx, dy)
            if r > self.max_m or r < 50:
                continue
            pd = self.pd if r < 0.8 * self.max_m else self.pd - (self.pd - 0.6) * (r - 0.8 * self.max_m) / (0.2 * self.max_m)
            if self.rnd.random() > pd:
                continue
            seen.add(tid)
            rm = max(1.0, r + self.rnd.gauss(0, self.range_sigma))
            az = (math.degrees(math.atan2(dx, dy)) + self.rnd.gauss(0, self.az_sigma)) % 360
            ex, ny = rm * math.sin(math.radians(az)), rm * math.cos(math.radians(az))
            tn, age = self._local(tid)
            obs.append(Observation(rm, az, ex, ny, alt, speed, course, ssr, m3a, tn, age, kind))
            meas.append({"x": self.east + ex, "y": self.north + ny, "alt": alt, "ssr": ssr, "mode3a": m3a,
                         "kind": kind, "radar": self.name, "sigma": self.range_sigma + r * math.radians(self.az_sigma)})
        for tid in [t for t in self.local_tracks if t not in seen and t not in {s[0] for s in snapshot}]:
            del self.local_tracks[tid]
        clutter = []
        for _ in range(self.rnd.randint(0, self.clutter)):
            rm = self.rnd.uniform(0.1, 0.9) * self.max_m
            az = self.rnd.uniform(0, 360)
            ex, ny = rm * math.sin(math.radians(az)), rm * math.cos(math.radians(az))
            clutter.append(Observation(rm, az, ex, ny, self.rnd.uniform(0, 300), 0.0, 0.0, False, 0, 0, 0, "clutter"))
            meas.append({"x": self.east + ex, "y": self.north + ny, "alt": None, "ssr": False, "mode3a": 0,
                         "kind": "clutter", "radar": self.name, "sigma": 50.0})
        return obs, clutter, meas


# ---------------------------------------------------------------- correlation and filtering

class Axis:
    """One axis of a constant-velocity Kalman filter: state (p, v), covariance (pp, pv, vv)."""

    def __init__(self, z, var, vmax):
        self.p, self.v = z, 0.0
        self.pp, self.pv, self.vv = var, 0.0, vmax * vmax

    def predicted(self, dt, q):
        p = self.p + self.v * dt
        pp = self.pp + 2 * dt * self.pv + dt * dt * self.vv + q * dt ** 4 / 4
        pv = self.pv + dt * self.vv + q * dt ** 3 / 2
        vv = self.vv + q * dt * dt
        return p, pp, pv, vv

    def update(self, z, var, dt, q):
        p, pp, pv, vv = self.predicted(dt, q)
        s = pp + var
        k0, k1 = pp / s, pv / s
        r = z - p
        self.p, self.v = p + k0 * r, self.v + k1 * r
        self.pp, self.pv, self.vv = (1 - k0) * pp, (1 - k0) * pv, vv - k1 * pv


class SystemTrack:
    """A system track: a Kalman filter per horizontal axis, smoothed altitude, the radars that fed it."""

    GATE = 16.0   # squared normalised distance (about 4 sigma) for association

    def __init__(self, number, m, t):
        self.number = number
        fast = m["kind"] == "projectile"
        self.q = (10.0 if fast else 6.0) ** 2          # manoeuvre: white acceleration variance (m/s^2)^2
        var = m["sigma"] ** 2
        vmax = 700.0 if fast else 250.0
        self.ax, self.ay = Axis(m["x"], var, vmax), Axis(m["y"], var, vmax)
        self.alt = m["alt"]
        self.mode3a = m["mode3a"] if m["ssr"] else None
        self.kinds = {m["kind"]: 1}
        self.t = self.last_update = t
        self.updates = 1
        self.radars = {m["radar"]: t}
        self.confirmed = False

    @property
    def x(self):
        return self.ax.p

    @property
    def y(self):
        return self.ay.p

    @property
    def vx(self):
        return self.ax.v

    @property
    def vy(self):
        return self.ay.v

    def predict(self, t):
        dt = t - self.t
        return self.ax.p + self.ax.v * dt, self.ay.p + self.ay.v * dt

    def distance2(self, m, t):
        """Squared normalised distance of a detection from the predicted position."""
        dt, var = max(0.0, t - self.t), m["sigma"] ** 2
        px, ppx, _, _ = self.ax.predicted(dt, self.q)
        py, ppy, _, _ = self.ay.predicted(dt, self.q)
        return (m["x"] - px) ** 2 / (ppx + var) + (m["y"] - py) ** 2 / (ppy + var)

    def update(self, m, t):
        dt, var = max(0.0, t - self.t), m["sigma"] ** 2
        self.ax.update(m["x"], var, dt, self.q)
        self.ay.update(m["y"], var, dt, self.q)
        if m["alt"] is not None:
            self.alt = m["alt"] if self.alt is None else 0.7 * self.alt + 0.3 * m["alt"]
        if m["ssr"]:
            self.mode3a = m["mode3a"]
        self.kinds[m["kind"]] = self.kinds.get(m["kind"], 0) + 1
        self.t = self.last_update = t
        self.updates += 1
        self.radars[m["radar"]] = t
        if self.updates >= 3:
            self.confirmed = True

    @property
    def kind(self):
        return max(self.kinds, key=self.kinds.get)

    def mono(self, t, window=5.0):
        return sum(1 for ts in self.radars.values() if t - ts <= window) < 2


class Tracker:
    """Correlates the radars' local tracks into system tracks.

    Input is each radar's tracked detections (its own plot-to-track processing has already
    rejected clutter, as a central track processor expects). Association is greedy global
    nearest neighbour on the squared normalised distance inside a 4-sigma gate; a detection
    outside every gate starts a tentative track. A track is confirmed after 3 updates (from any
    radars), coasts on its filter, and is dropped after `coast_s` without an update (tentative
    tracks after `tentative_s`, projectiles after 3 s)."""

    def __init__(self, coast_s=8.0, tentative_s=3.0):
        self.coast_s, self.tentative_s = coast_s, tentative_s
        self.lock = threading.Lock()
        self.tracks = []
        self.next_number = 1
        self.counts = {"detections": 0, "associated": 0, "initiated": 0, "dropped": 0}

    def ingest(self, meas, t=None):
        t = time.time() if t is None else t
        meas = [m for m in meas if m["kind"] != "clutter"]
        with self.lock:
            self.counts["detections"] += len(meas)
            pairs = []
            for i, m in enumerate(meas):
                for tr in self.tracks:
                    d2 = tr.distance2(m, t)
                    if d2 <= SystemTrack.GATE:
                        pairs.append((d2 + (0 if tr.confirmed else 4), i, tr))
            pairs.sort(key=lambda p: p[0])
            used_m, used_t = set(), set()
            for _, i, tr in pairs:
                if i in used_m or id(tr) in used_t:
                    continue
                tr.update(meas[i], t)
                used_m.add(i)
                used_t.add(id(tr))
                self.counts["associated"] += 1
            for i, m in enumerate(meas):
                if i not in used_m:
                    self.tracks.append(SystemTrack(self.next_number, m, t))
                    self.next_number = self.next_number % 4095 + 1
                    self.counts["initiated"] += 1
            self.prune(t)
            self.merge()

    def merge(self):
        """Two confirmed tracks on the same target (inside each other's gate, same heading) become one."""
        keep = []
        for tr in sorted(self.tracks, key=lambda x: -x.updates):
            dup = False
            for k in keep:
                if not (k.confirmed and tr.confirmed):
                    continue
                dx, dy = tr.x - k.x, tr.y - k.y
                s = k.ax.pp + tr.ax.pp + k.ay.pp + tr.ay.pp
                dv = math.hypot(tr.vx - k.vx, tr.vy - k.vy)
                if (dx * dx + dy * dy) <= SystemTrack.GATE * s / 2 and dv < 30:
                    dup = True
                    for r, ts in tr.radars.items():
                        k.radars[r] = max(ts, k.radars.get(r, 0))
                    break
            if not dup:
                keep.append(tr)
        self.tracks = keep

    def prune(self, t):
        keep = []
        for tr in self.tracks:
            limit = self.coast_s if tr.confirmed else self.tentative_s
            if tr.kind == "projectile":
                limit = min(limit, 3.0)
            if t - tr.last_update <= limit:
                keep.append(tr)
            else:
                self.counts["dropped"] += 1
        self.tracks = keep

    def confirmed(self, t=None):
        t = time.time() if t is None else t
        with self.lock:
            self.prune(t)
            out = []
            for tr in self.tracks:
                if not tr.confirmed:
                    continue
                x, y = tr.predict(t)
                out.append({"number": tr.number, "x": x, "y": y, "vx": tr.vx, "vy": tr.vy, "alt": tr.alt,
                            "mode3a": tr.mode3a, "mono": tr.mono(t), "kind": tr.kind,
                            "age_s": t - tr.last_update})
            return out


# ---------------------------------------------------------------- ASTERIX CAT062 and CAT034 position

def cat062_track(sac, sic, t, trk, frame, service_id=1, tod=None, record=None):
    """One CAT062 system track record (Edition 1.18 UAP)."""
    lat, lon = frame.to_wgs84(trk["x"], trk["y"])
    status = ((0x80 if trk["mono"] else 0x00)          # MON: 1 = monosensor
              | (2 << 3)                                # SRC: 3D radar
              | 0x01)                                   # FX: second octet follows
    items = {
        1: bytes([sac, sic]),                                                    # I062/010
        3: bytes([service_id & 0xFF]),                                           # I062/015
        4: tod(t),                                                               # I062/070
        5: struct.pack(">ii", wgs84_25(lat), wgs84_25(lon)),                    # I062/105
        6: (int(round(trk["x"] * 2)) & 0xFFFFFF).to_bytes(3, "big")
           + (int(round(trk["y"] * 2)) & 0xFFFFFF).to_bytes(3, "big"),            # I062/100, 0.5 m
        7: struct.pack(">hh", max(-32768, min(32767, int(round(trk["vx"] * 4)))),
                       max(-32768, min(32767, int(round(trk["vy"] * 4))))),       # I062/185, 0.25 m/s
        12: struct.pack(">H", trk["number"] & 0x0FFF),                           # I062/040
        13: bytes([status, 0x80]),                                               # I062/080 + SIM
    }
    if trk.get("mode3a") is not None:
        items[9] = struct.pack(">H", trk["mode3a"] & 0x0FFF)                     # I062/060
    if trk.get("alt") is not None:
        items[18] = struct.pack(">h", max(-32768, min(32767, int(round(trk["alt"] / FT_M / 6.25)))))  # I062/130
    return record(items)


def i034_120(height_m, lat, lon):
    """I034/120 3D position of the data source: height (m), latitude, longitude (180/2^23 deg)."""
    return struct.pack(">h", int(round(height_m))) + wgs84_23(lat).to_bytes(3, "big") + wgs84_23(lon).to_bytes(3, "big")
