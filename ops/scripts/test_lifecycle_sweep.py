#!/usr/bin/env python3
import copy
import importlib.util
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("lifecycle_sweep", HERE / "lifecycle-sweep.py")
sweep = importlib.util.module_from_spec(spec)
sys.modules["lifecycle_sweep"] = sweep
spec.loader.exec_module(sweep)

val_spec = importlib.util.spec_from_file_location("validate_hold_records", HERE.parent / "lifecycle" / "validate-hold-records.py")
validator = importlib.util.module_from_spec(val_spec)
sys.modules["validate_hold_records"] = validator
val_spec.loader.exec_module(validator)

def make_time(days_ago):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - 86400 * days_ago))

class MockNetBird:
    def __init__(self):
        self.peers = []
        self.groups = []
        self.keys = []
        self.calls = []

    def api(self, path, method="GET", body=None, token=""):
        self.calls.append((method, path, body))
        if method == "GET":
            # Copies, as the real API returns: a snapshot the code keeps is
            # NOT live-updated by later writes, so a stale read-modify-write
            # shows up as a lost update here exactly as it would on NetBird.
            if path == "/api/peers":
                return copy.deepcopy(self.peers)
            if path == "/api/groups":
                return copy.deepcopy(self.groups)
            if path == "/api/setup-keys":
                return copy.deepcopy(self.keys)
            if path.startswith("/api/peers/"):
                pid = path.split("/")[-1]
                for p in self.peers:
                    if str(p.get("id")) == pid:
                        return copy.deepcopy(p)
                raise Exception("peer not found")
            if path.startswith("/api/groups/"):
                gid = path.split("/")[-1]
                for g in self.groups:
                    if str(g.get("id")) == gid:
                        return copy.deepcopy(g)
                raise Exception("group not found")
            if path.startswith("/api/setup-keys/"):
                kid = path.split("/")[-1]
                for k in self.keys:
                    if str(k.get("id")) == kid:
                        return k
                raise Exception("key not found")
        elif method == "PUT":
            if path.startswith("/api/groups/"):
                gid = path.split("/")[-1]
                for g in self.groups:
                    if str(g.get("id")) == gid:
                        g["peers"] = body.get("peers") or []
                        return g
            if path.startswith("/api/setup-keys/"):
                kid = path.split("/")[-1]
                for k in self.keys:
                    if str(k.get("id")) == kid:
                        k.update(body)
                        return k
        elif method == "DELETE":
            pass
        return {}



def full_rec(d):
    """A complete v1 hold record around the fields a test cares about; the
    sweep skips a line missing any required field, so partial records would
    read as bad lines."""
    rec = {"v": 1, "lane": "dormancy", "site": "zagros", "peer_name": "peer-%s" % d.get("peer_id", "x"),
           "handle": None, "entered_at": "2026-09-01T00:00:00Z", "prior_groups": [], "actor": "sweep"}
    rec.update(d)
    return json.dumps(rec)


class LifecycleSweepTest(unittest.TestCase):
    def setUp(self):
        self.nb = MockNetBird()
        self.patch_api = patch("lifecycle_sweep.api", side_effect=self.nb.api)
        self.patch_api.start()

        self.temp_dir = tempfile.TemporaryDirectory()
        self.d = Path(self.temp_dir.name)
        self.ledger = self.d / "ledger.log"
        self.hold_records = self.d / "hold_records.jsonl"
        
        self.env_patcher = patch.dict(os.environ, {
            # An explicit, absent membership file: these tests are the
            # ungoverned behaviour whatever /etc/goat or the checkout holds.
            "LIFECYCLE_MEMBERSHIP_FILE": str(Path(tempfile.gettempdir()) / "no-such-membership.yaml"),
            "NB_TOKEN": "fake-token",
            "LIFECYCLE_LEDGER": str(self.ledger),
            "LIFECYCLE_HOLD_RECORDS": str(self.hold_records),
            "GOATNET": "zagros"
        })
        self.env_patcher.start()
        
        sweep.SITE = "zagros"
        sweep.HOLD_RECORDS = str(self.hold_records)
        sweep.LEDGER = str(self.ledger)

    def tearDown(self):
        self.env_patcher.stop()
        self.patch_api.stop()
        self.temp_dir.cleanup()

    def test_hold_record_writers_close_lock_on_all_exit_paths(self):
        import builtins
        actions = [
            lambda: sweep.append_hold_record({"id": "fixture", "name": "fixture"},
                                             "LAB-PROBATION-GEAR", [], "test", None),
            lambda: sweep.mark_hold_record_releasing("fixture"),
            lambda: sweep.delete_hold_record("fixture"),
        ]
        for action in actions:
            for failure in (None, "acquire", "read", "release"):
                with self.subTest(action=actions.index(action), failure=failure):
                    handles, events = [], []
                    def opened(*args, **kwargs):
                        handle = builtins.open(*args, **kwargs)
                        handles.append(handle)
                        return handle
                    def flock(fd, operation):
                        self.assertFalse(handles[-1].closed)
                        events.append(operation)
                        if ((failure == "acquire" and operation == sweep.fcntl.LOCK_EX)
                                or (failure == "release" and operation == sweep.fcntl.LOCK_UN)):
                            raise OSError("injected " + failure)
                    def read():
                        self.assertEqual(events, [sweep.fcntl.LOCK_EX])
                        self.assertFalse(handles[-1].closed)
                        if failure == "read":
                            raise OSError("injected read")
                        return [], []
                    with patch.object(sweep, "open", side_effect=opened, create=True), \
                            patch.object(sweep.fcntl, "flock", side_effect=flock), \
                            patch.object(sweep, "_read_hold_lines", side_effect=read), \
                            patch.object(sweep, "_rewrite_hold_file"):
                        if failure:
                            with self.assertRaisesRegex(OSError, "injected " + failure):
                                action()
                        else:
                            action()
                    self.assertEqual(len(handles), 1)
                    self.assertTrue(handles[0].closed)
                    expected = [sweep.fcntl.LOCK_EX]
                    if failure != "acquire":
                        expected.append(sweep.fcntl.LOCK_UN)
                    self.assertEqual(events, expected)

    def run_sweep(self, *args):
        with patch("sys.argv", ["lifecycle-sweep.py"] + list(args)):
            return sweep.main()

    def test_json_report_shape(self):
        import io
        from contextlib import redirect_stdout

        # Provide minimal non-empty peers/groups so the sweep treats the
        # NetBird API as reachable but produces empty lanes.
        self.nb.groups = [
            {"id": "gAll", "name": "All", "peers": []},
        ]
        self.nb.peers = [
            {
                "id": "p1",
                "name": "peer-ignored-zagros-2026-09-01",
                "connected": True,
                "last_seen": make_time(1),
                "groups": [self.nb.groups[0]],
            }
        ]

        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--json")

        self.assertEqual(rc, 0)
        data = json.loads(out.getvalue())
        for key in [
            "generated",
            "thresholds",
            "quarantine",
            "revoke",
            "purge",
            "unageable",
            "refused",
        ]:
            self.assertIn(key, data)
        self.assertIsInstance(data["thresholds"], dict)
        self.assertIn("person", data["thresholds"])
        self.assertIn("gear", data["thresholds"])
        self.assertIn("purge_grace", data["thresholds"])

    def test_one_role_peer_quarantine(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {
                "id": "p1", 
                "name": "peer-bob-zagros-2026-09-01", 
                "connected": False, 
                "last_seen": make_time(15), 
                "groups": [self.nb.groups[0], self.nb.groups[2]]
            }
        ]
        
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
            
        g2_peers = [p for p in self.nb.groups if p["id"] == "g2"][0]["peers"]
        self.assertIn("p1", [x if isinstance(x, str) else x["id"] for x in g2_peers])
        g1_peers = [p for p in self.nb.groups if p["id"] == "g1"][0]["peers"]
        self.assertNotIn("p1", [x if isinstance(x, str) else x["id"] for x in g1_peers])
        
        self.assertEqual(validator.validate_file(str(self.hold_records)), [])
        with open(self.hold_records) as fh:
            rec = json.loads(fh.readline())
            self.assertEqual(rec["hold_group"], "ZAGROS-PROBATION-DEVELOPER")
            self.assertEqual(rec["prior_groups"], ["ZAGROS-DEVELOPER"])

    def test_gear_peer_quarantine(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-SENSOR", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-GEAR", "peers": []}
        ]
        self.nb.peers = [
            {
                "id": "p1", 
                "name": "peer-sensor-zagros-2026-09-01", 
                "connected": False, 
                "last_seen": make_time(35),
                "groups": [self.nb.groups[0]]
            }
        ]
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        
        self.assertEqual(validator.validate_file(str(self.hold_records)), [])
        with open(self.hold_records) as fh:
            rec = json.loads(fh.readline())
            self.assertEqual(rec["hold_group"], "ZAGROS-PROBATION-GEAR")

    def test_refusal_rules(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}, {"id": "p3"}]},
            {"id": "g2", "name": "ZAGROS-OBSERVER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p3"}]},
            {"id": "g4", "name": "ZAGROS-PROBATION-OBSERVER", "peers": []},
            {"id": "g5", "name": "UNKNOWN", "peers": [{"id": "p2"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "two-roles", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1]]},
            {"id": "p2", "name": "no-roles", "connected": False, "last_seen": make_time(35), "groups": [self.nb.groups[4]]},
            {"id": "p3", "name": "already-held-and-role", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[2]]},
        ]
        
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        
        data = json.loads(out.getvalue())
        refused = {r["name"]: r for r in data["refused"]}
        
        self.assertIn("multiple role groups with mirrors", refused["two-roles"]["reason"])
        self.assertIn("ZAGROS-DEVELOPER", refused["two-roles"]["reason"])
        self.assertIn("ZAGROS-OBSERVER", refused["two-roles"]["reason"])
        
        self.assertIn("no role group with mirror and no gear class group", refused["no-roles"]["reason"])
        
        self.assertIn("in hold group(s) ZAGROS-PROBATION-DEVELOPER and group(s) ZAGROS-DEVELOPER", refused["already-held-and-role"]["reason"])

    def test_role_group_without_probation_mirror_refused(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-MONITOR", "peers": [{"id": "p1"}]},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {
                "id": "p1",
                "name": "peer-monitor-zagros-2026-09-01",
                "connected": False,
                "last_seen": make_time(35),
                "groups": [self.nb.groups[0], self.nb.groups[1]],
            }
        ]

        import io
        from contextlib import redirect_stdout

        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--json")

        self.assertEqual(rc, 0)
        data = json.loads(out.getvalue())
        self.assertEqual(len(data["quarantine"]), 0)
        self.assertEqual(len(data["refused"]), 1)
        row = data["refused"][0]
        self.assertEqual(row["name"], "peer-monitor-zagros-2026-09-01")
        self.assertIn("role group(s) ZAGROS-MONITOR have no PROBATION mirror", row["reason"])

    def test_already_held_no_role_silent_skip(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {
                "id": "p1",
                "name": "peer-bob-zagros-2026-09-01",
                "connected": False,
                "last_seen": make_time(15),
                "groups": [self.nb.groups[0], self.nb.groups[1]],
            }
        ]
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        data = json.loads(out.getvalue())
        self.assertEqual(len(data["refused"]), 0)
        self.assertEqual(len(data["quarantine"]), 0)

    def test_realistic_actor_string_validation(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {
                "id": "p1",
                "name": "peer-bob-zagros-2026-09-01",
                "connected": False,
                "last_seen": make_time(15),
                "groups": [self.nb.groups[0], self.nb.groups[2]],
            }
        ]
        
        # Test realistic mixed-case human free text actor
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "John Doe (Helpdesk)"), 0)
        
        self.assertEqual(validator.validate_file(str(self.hold_records)), [])
        with open(self.hold_records) as fh:
            rec = json.loads(fh.readline())
            self.assertEqual(rec["actor"], "John Doe (Helpdesk)")

    def test_adds_before_removing(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0]]}
        ]
        
        self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        
        put_calls = [c for c in self.nb.calls if c[0] == "PUT"]
        self.assertGreaterEqual(len(put_calls), 2)
        self.assertTrue(put_calls[0][1].endswith("/g2"), "First PUT must be to the hold group")
        self.assertTrue(put_calls[1][1].endswith("/g1"), "Second PUT must be to the prior group")

    def test_stale_hold_record_does_not_block_rehold(self):
        # First run: peer is held into PROBATION, creating a hold-record line.
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {
                "id": "p1",
                "name": "peer-bob-zagros-2026-09-01",
                "connected": False,
                "last_seen": make_time(15),
                "groups": [self.nb.groups[0], self.nb.groups[2]],
            }
        ]

        self.assertEqual(
            self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"),
            0,
        )

        # Simulate an out-of-band release: peer back in the role group,
        # not in the probation group, but the hold-record line remains.
        self.nb.groups[0]["peers"] = [{"id": "p1"}]
        self.nb.groups[1]["peers"] = []
        self.nb.peers[0]["groups"] = [self.nb.groups[0], self.nb.groups[2]]

        # Second run must re-hold the peer instead of treating it as done
        # purely because a stale hold-record line exists.
        self.assertEqual(
            self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"),
            0,
        )

        # The peer is back in the probation group and out of the role group.
        g_dev = [g for g in self.nb.groups if g["name"] == "ZAGROS-DEVELOPER"][0]
        g_prob = [g for g in self.nb.groups if g["name"] == "ZAGROS-PROBATION-DEVELOPER"][0]
        dev_peers = [p if isinstance(p, str) else p.get("id") for p in g_dev.get("peers") or []]
        prob_peers = [p if isinstance(p, str) else p.get("id") for p in g_prob.get("peers") or []]
        self.assertNotIn("p1", dev_peers)
        self.assertIn("p1", prob_peers)

        # Hold records remain valid and contain exactly one line for this peer.
        self.assertEqual(validator.validate_file(str(self.hold_records)), [])
        with open(self.hold_records) as fh:
            lines = fh.readlines()
        ids = []
        for line in lines:
            data = json.loads(line)
            if data.get("peer_id") == "p1":
                ids.append(data["peer_id"])
        self.assertEqual(ids, ["p1"])

    def test_revoke_deletes_hold_record(self):
        self.nb.groups = [{"id": "g1", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]}]
        self.nb.peers = [{"id": "p1", "name": "peer-bob", "connected": False, "last_seen": make_time(65), "groups": [self.nb.groups[0]]}]
        
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p2", "hold_group": "ZAGROS-PROBATION-OBSERVER", "prior_groups": ["ZAGROS-OBSERVER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")
            fh.write(full_rec({"peer_id": "p1", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")
            
        self.assertEqual(self.run_sweep("--apply", "--lane", "revoke", "--actor", "ops"), 0)
        
        with open(self.hold_records) as fh:
            lines = fh.readlines()
            self.assertEqual(len(lines), 1)
            rec = json.loads(lines[0])
            self.assertEqual(rec["peer_id"], "p2")

    def test_quarantined_device_is_exempt_through_its_hold_record(self):
        """ADR 1216: a switch quarantined before the rule holds only its hold
        group; its class is in the record's prior_groups, and the revoke lane
        must still leave it alone."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "gP", "name": "ZAGROS-PROBATION-GEAR", "peers": [{"id": "p1"}, {"id": "p2"}]},
            {"id": "gN", "name": "ZAGROS-NETWORK", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "machine-switch-1", "connected": False,
             "last_seen": make_time(130), "groups": [self.nb.groups[0]]},
            # Same live shape, but its record says it was a sensor: revokable.
            {"id": "p2", "name": "machine-sensor-1", "connected": False,
             "last_seen": make_time(130), "groups": [self.nb.groups[0]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "hold_group": "ZAGROS-PROBATION-GEAR",
                               "prior_groups": ["ZAGROS-NETWORK"]}) + "\n")
            fh.write(full_rec({"peer_id": "p2", "hold_group": "ZAGROS-PROBATION-GEAR",
                               "prior_groups": ["ZAGROS-SENSOR-FEED"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        data = json.loads(out.getvalue())
        exempt = {r["id"]: r for r in data["exempt"]}
        self.assertEqual(set(exempt), {"p1"})
        self.assertEqual(exempt["p1"]["exempt_class"], "NETWORK")
        self.assertEqual(exempt["p1"]["would_be"], "revoke")
        self.assertEqual({r["id"] for r in data["revoke"]}, {"p2"})
        self.nb.calls.clear()
        with redirect_stdout(io.StringIO()):
            self.run_sweep("--apply", "--lane", "revoke", "--actor", "ops")
        touched = [c for c in self.nb.calls if c[0] != "GET" and "p1" in json.dumps(c[1:])]
        self.assertEqual(touched, [], "revoke lane acted on a held switch: %s" % touched)

    def test_dual_homed_device_is_refused_not_exempt(self):
        """A device in a hold group AND its class group is inconsistent live
        state; it must surface as refused, not vanish into the exempt bucket."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "gN", "name": "ZAGROS-NETWORK", "peers": [{"id": "p1"}]},
            {"id": "gP", "name": "ZAGROS-PROBATION-GEAR", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "machine-switch-1", "connected": False,
             "last_seen": make_time(40), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        data = json.loads(out.getvalue())
        self.assertEqual(data["exempt"], [])
        refused = {r["id"]: r for r in data["refused"]}
        self.assertIn("p1", refused)
        self.assertTrue(sweep.is_inconsistent(refused["p1"]["reason"]))

    def test_net_engineer_laptop_is_a_person(self):
        """ADR 1216: NET-ENGINEER is a person group, so a laptop holding only
        it is on the person clock, not the gear clock."""
        self.assertEqual(sweep.classify({"groups": [{"name": "SITE_ONE-NET-ENGINEER"}]}), "person")
        self.assertEqual(sweep.classify({"groups": [{"name": "SITE_ONE-NET-ENGINEER"},
                                                    {"name": "SITE_ONE-NETWORK"}]}), "gear")

    def test_device_classes_are_reported_but_never_quarantined_or_revoked(self):
        """ADR 1216: NETWORK / CONTROL / EDGE-SERVER peers are explicit-revocation only."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "gN", "name": "SITE_ONE-NETWORK", "peers": [{"id": "p1"}]},
            {"id": "gC", "name": "ZAGROS-PROD-CONTROL", "peers": [{"id": "p2"}]},
            {"id": "gE", "name": "SITE_ONE-EDGE-SERVER", "peers": [{"id": "p3"}]},
            {"id": "gJ", "name": "ZAGROS-NETWORK-JUMP", "peers": [{"id": "p4"}]},
            {"id": "gP", "name": "ZAGROS-PROBATION-GEAR", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "machine-switch-1", "connected": False,
             "last_seen": make_time(200), "groups": [self.nb.groups[0]]},
            {"id": "p2", "name": "machine-c2-1", "connected": False,
             "last_seen": make_time(40), "groups": [self.nb.groups[1]]},
            {"id": "p3", "name": "machine-edge-1", "connected": False,
             "last_seen": make_time(130), "groups": [self.nb.groups[2]]},
            # NETWORK-JUMP is NOT exempt: the suffix match must not catch it.
            {"id": "p4", "name": "machine-jump-1", "connected": False,
             "last_seen": make_time(130), "groups": [self.nb.groups[3]]},
        ]

        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        data = json.loads(out.getvalue())
        exempt = {r["id"]: r for r in data["exempt"]}
        self.assertEqual(set(exempt), {"p1", "p2", "p3"})
        self.assertEqual(exempt["p1"]["exempt_class"], "NETWORK")
        self.assertEqual(exempt["p1"]["would_be"], "revoke")
        self.assertEqual(exempt["p2"]["would_be"], "quarantine")
        self.assertEqual(exempt["p3"]["exempt_class"], "EDGE-SERVER")
        laned = {r["id"] for r in data["quarantine"] + data["revoke"]}
        self.assertNotIn("p1", laned)
        self.assertNotIn("p2", laned)
        self.assertNotIn("p3", laned)
        self.assertIn("p4", laned)

        # --apply on both lanes writes nothing for the exempt peers.
        for lane in ("quarantine", "revoke"):
            self.nb.calls.clear()
            with redirect_stdout(io.StringIO()):
                self.run_sweep("--apply", "--lane", lane, "--actor", "ops")
            touched = [c for c in self.nb.calls if c[0] != "GET"
                       and any(pid in json.dumps(c[1:]) for pid in ("p1", "p2", "p3"))]
            self.assertEqual(touched, [], "lane %s acted on an exempt peer: %s" % (lane, touched))
        # Group membership of the exempt peers is unchanged.
        for gid, pid in (("gN", "p1"), ("gC", "p2"), ("gE", "p3")):
            g = [x for x in self.nb.groups if x["id"] == gid][0]
            self.assertIn(pid, [x if isinstance(x, str) else x["id"] for x in g["peers"]])

        # The human report names the bucket.
        out = io.StringIO()
        with redirect_stdout(out):
            self.run_sweep()
        self.assertIn("EXEMPT", out.getvalue())
        self.assertIn("explicit revocation only", out.getvalue())

    def test_apply_without_actor_refused(self):
        import io
        from contextlib import redirect_stderr
        out = io.StringIO()
        with redirect_stderr(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine"), 2)
            self.assertEqual(self.run_sweep("--apply", "--lane", "revoke"), 2)
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1"), 2)
        err = out.getvalue()
        self.assertIn("--apply requires --actor", err)

    def test_apply_requires_hold_records_path_for_mutating_lanes(self):
        import io
        from contextlib import redirect_stderr

        # Simulate a hand-run without LIFECYCLE_HOLD_RECORDS set.
        sweep.HOLD_RECORDS = ""

        out = io.StringIO()
        with redirect_stderr(out):
            self.assertEqual(
                self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"),
                2,
            )
            self.assertEqual(
                self.run_sweep(
                    "--apply",
                    "--lane",
                    "release",
                    "--peer",
                    "p1",
                    "--actor",
                    "ops",
                ),
                2,
            )
            self.assertEqual(
                self.run_sweep("--apply", "--lane", "revoke", "--actor", "ops"),
                2,
            )

        err = out.getvalue()
        self.assertIn("LIFECYCLE_HOLD_RECORDS", err)
        self.assertIn("/etc/goat/lifecycle-sweep.env", err)

    def test_release_lane(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p2"}]},
            {"id": "g4", "name": "ZAGROS-OBSERVER", "peers": []}
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-record", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[1]]},
            {"id": "p2", "name": "peer-ledger", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[2]]},
        ]
        
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")
            
        with open(self.ledger, "w") as fh:
            fh.write("2026-09-01T00:00:00Z\tquarantine\tpeer-ledger\tp2\tZAGROS-OBSERVER\n")
            
        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops"), 0)
        
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")
            
        g1_peers = [p for p in self.nb.groups if p["id"] == "g1"][0]["peers"]
        self.assertIn("p1", [x if isinstance(x, str) else x["id"] for x in g1_peers])
        g2_peers = [p for p in self.nb.groups if p["id"] == "g2"][0]["peers"]
        self.assertNotIn("p1", [x if isinstance(x, str) else x["id"] for x in g2_peers])
        
        # Second run on the same peer: the mesh now shows it back in its role
        # group and the sweep's own ledger still carries the hold's row (it
        # always will — the ledger is append-only). That row must not turn
        # the repeat into a "may be a security containment" refusal: the
        # peer is not in QUARANTINED, so there is nothing to release.
        self.nb.peers[0]["groups"] = [self.nb.groups[0]]
        with open(self.ledger, "a") as fh:
            fh.write("2026-09-02T00:00:00Z\tquarantine\tpeer-record\tp1\tZAGROS-DEVELOPER\n")
        self.nb.calls = []
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops"), 0)

        self.assertIn("no hold record for peer-record and it is not in ZAGROS-QUARANTINED \u2014 nothing to release.", out.getvalue())
        self.assertNotIn("security containment", out.getvalue())
        self.assertNotIn("REFUSED", out.getvalue())
        put_calls = [c for c in self.nb.calls if c[0] == "PUT"]
        self.assertEqual(len(put_calls), 0)
        
        # No record: the ledger fallback is refused until the operator attests
        # the QUARANTINED membership is a dormancy hold, not a containment.
        self.nb.calls = []
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p2", "--actor", "ops"), 1)
        self.assertIn("REFUSED peer-ledger", out.getvalue())
        self.assertIn("--acknowledge-quarantined-is-dormancy", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p2", "--actor", "ops",
                                        "--acknowledge-quarantined-is-dormancy"), 0)
        g4_peers = [p for p in self.nb.groups if p["id"] == "g4"][0]["peers"]
        self.assertIn("p2", [x if isinstance(x, str) else x["id"] for x in g4_peers])
        g3_peers = [p for p in self.nb.groups if p["id"] == "g3"][0]["peers"]
        self.assertNotIn("p2", [x if isinstance(x, str) else x["id"] for x in g3_peers])
        
        # Idempotent second run for peer-ledger
        # Simulate NetBird API updating the peer's group list after group PUT
        self.nb.peers[1]["groups"] = [self.nb.groups[3]] # now in g4 (OBSERVER)
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p2", "--actor", "ops",
                                            "--acknowledge-quarantined-is-dormancy"), 0)
        self.assertIn("no hold record for peer-ledger and it is not in ZAGROS-QUARANTINED", out.getvalue())

    def test_release_refuses_when_exact_hold_identity_changed(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [{
            "id": "p1", "name": "peer-record", "connected": False,
            "last_seen": make_time(15), "groups": [self.nb.groups[1]],
        }]
        self.hold_records.write_text(full_rec({
            "peer_id": "p1", "hold_group": "ZAGROS-PROBATION-DEVELOPER",
            "prior_groups": ["ZAGROS-DEVELOPER"],
            "entered_at": "2026-10-20T00:00:00Z",
        }) + "\n")
        self.nb.calls = []
        self.assertEqual(self.run_sweep(
            "--apply", "--lane", "release", "--peer", "p1", "--actor", "ops",
            "--hold-entered-at", "2026-10-19T00:00:00Z"), 1)
        self.assertEqual([c for c in self.nb.calls if c[0] != "GET"], [])
        self.assertIn("2026-10-20T00:00:00Z", self.hold_records.read_text())


    def test_release_lane_with_peer_name(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-record-name", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[1]]},
        ]
        
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-record-name", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")
            
        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "peer-record-name", "--actor", "ops"), 0)
        
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")
            
        g1_peers = [p for p in self.nb.groups if p["id"] == "g1"][0]["peers"]
        self.assertIn("p1", [x if isinstance(x, str) else x["id"] for x in g1_peers])
        g2_peers = [p for p in self.nb.groups if p["id"] == "g2"][0]["peers"]
        self.assertNotIn("p1", [x if isinstance(x, str) else x["id"] for x in g2_peers])


    def test_real_caller_full_field_set(self):
        # A test driven with a realistic NetBird API payload (all fields) to ensure
        # the code tolerates and correctly handles the REAL full JSON.
        self.nb.groups = [
            {
                "id": "ch8i4ug6lnn4g9hqv7m0",
                "name": "ZAGROS-DEVELOPER",
                "peers_count": 1,
                "issued": "api",
                "peers": [{"id": "ch8i4ug6lnn4g9hqv7mg", "name": "developer-laptop"}]
            },
            {
                "id": "ch8i4ug6lnn4g9hqv7m1",
                "name": "ZAGROS-PROBATION-DEVELOPER",
                "peers_count": 0,
                "issued": "api",
                "peers": []
            }
        ]
        self.nb.peers = [
            {
                "id": "ch8i4ug6lnn4g9hqv7mg",
                "name": "developer-laptop-zagros-2026",
                "ip": "100.127.0.1",
                "connected": False,
                "last_seen": make_time(15),
                "os": "darwin",
                "version": "0.22.0",
                "groups": [self.nb.groups[0]],
                "network_addresses": [{"network": "100.64.0.0/10"}],
                "login_expiration_enabled": False,
                "ssh_enabled": False,
                "approval_required": False
            }
        ]
        
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "Human Operator"), 0)
        
        g1_peers = [p for p in self.nb.groups if p["id"] == "ch8i4ug6lnn4g9hqv7m0"][0]["peers"]
        self.assertNotIn("ch8i4ug6lnn4g9hqv7mg", [x if isinstance(x, str) else x.get("id") for x in g1_peers])
        g2_peers = [p for p in self.nb.groups if p["id"] == "ch8i4ug6lnn4g9hqv7m1"][0]["peers"]
        self.assertIn("ch8i4ug6lnn4g9hqv7mg", [x if isinstance(x, str) else x.get("id") for x in g2_peers])

    def test_release_name_matched_record_is_deleted(self):
        # The peer re-enrolled under a new id; the record carries the old id
        # and matches by name only. Release must delete the record's line,
        # not look for one under the new id and leave a phantom hold behind.
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "pNEW"}]},
        ]
        self.nb.peers = [
            {"id": "pNEW", "name": "peer-bob-zagros-2026-09-01", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "pOLD", "peer_name": "peer-bob-zagros-2026-09-01", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")

        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pNEW", "--actor", "ops"), 0)

        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")
        g1_peers = [p for p in self.nb.groups if p["id"] == "g1"][0]["peers"]
        self.assertIn("pNEW", [x if isinstance(x, str) else x["id"] for x in g1_peers])

    def test_release_refuses_peer_in_hold_and_role_group(self):
        # Contradictory live state is refused, never resolved by restoring
        # on top of it (membership-lifecycle-and-transfer.md, "Lane and
        # group agree").
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-OBSERVER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-OBSERVER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")

        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("REFUSED", out.getvalue())
        self.assertIn("ZAGROS-PROBATION-DEVELOPER", out.getvalue())
        self.assertIn("ZAGROS-DEVELOPER", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        with open(self.hold_records) as fh:
            self.assertEqual(len(fh.readlines()), 1)

    def test_malformed_line_does_not_break_revoke_or_release(self):
        junk = "not json\n"
        # Revoke: the peer is deleted AND its record line goes; the junk
        # line is carried through for the validator to report.
        self.nb.groups = [{"id": "g1", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]}]
        self.nb.peers = [{"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False, "last_seen": make_time(65), "groups": [self.nb.groups[0]]}]
        with open(self.hold_records, "w") as fh:
            fh.write(junk)
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")
        self.assertEqual(self.run_sweep("--apply", "--lane", "revoke", "--actor", "ops"), 0)
        self.assertEqual([c for c in self.nb.calls if c[0] == "DELETE"], [("DELETE", "/api/peers/p1", None)])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), junk)

        # Release: same junk line, the release still completes.
        self.nb = MockNetBird()
        self.patch_api.stop()
        self.patch_api = patch("lifecycle_sweep.api", side_effect=self.nb.api)
        self.patch_api.start()
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p2"}]},
        ]
        self.nb.peers = [{"id": "p2", "name": "peer-ann-zagros-2026-09-01", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[1]]}]
        with open(self.hold_records, "w") as fh:
            fh.write(junk)
            fh.write(full_rec({"peer_id": "p2", "peer_name": "peer-ann-zagros-2026-09-01", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"], "v": 1, "lane": "dormancy", "site": "zagros"}) + "\n")
        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p2", "--actor", "ops"), 0)
        g1_peers = [p for p in self.nb.groups if p["id"] == "g1"][0]["peers"]
        self.assertIn("p2", [x if isinstance(x, str) else x["id"] for x in g1_peers])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), junk)

    def test_rewrite_preserves_file_mode(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
        ]
        # New file: readable by every ADMIN, not narrowed to the temp file's 0600.
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        self.assertEqual(os.stat(self.hold_records).st_mode & 0o777, 0o644)
        # Existing file: its mode survives a rewrite.
        os.chmod(self.hold_records, 0o660)
        alternate_groups = [gid for gid in os.getgroups() if gid != os.getgid()]
        expected_gid = os.stat(self.hold_records).st_gid
        if alternate_groups:
            expected_gid = alternate_groups[0]
            os.chown(self.hold_records, -1, expected_gid)
        self.assertIsNotNone(sweep.delete_hold_record("p1"))
        rewritten = os.stat(self.hold_records)
        self.assertEqual(rewritten.st_mode & 0o777, 0o660)
        self.assertEqual(rewritten.st_gid, expected_gid)

    def test_lane_release_without_apply_says_so(self):
        import io
        from contextlib import redirect_stderr, redirect_stdout
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "peer-x", "connected": True, "last_seen": make_time(1), "groups": [self.nb.groups[0]]}]
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            self.assertEqual(self.run_sweep("--lane", "release", "--json"), 0)
        self.assertIn("--lane release only acts with --apply", err.getvalue())
        json.loads(out.getvalue())  # stdout stays clean JSON

    def test_apply_with_whitespace_actor_refused(self):
        import io
        from contextlib import redirect_stderr
        out = io.StringIO()
        with redirect_stderr(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "   "), 2)
        err = out.getvalue()
        self.assertIn("--apply requires --actor", err)

    def test_gear_group_without_probation_mirror_refused(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-SENSOR", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {
                "id": "p1", 
                "name": "peer-sensor", 
                "connected": False, 
                "last_seen": make_time(35),
                "groups": [self.nb.groups[0]]
            }
        ]
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        
        data = json.loads(out.getvalue())
        self.assertEqual(len(data["quarantine"]), 0)
        self.assertEqual(len(data["refused"]), 1)
        self.assertIn("no ZAGROS-PROBATION-GEAR group on this goatnet", data["refused"][0]["reason"])

    def test_empty_groups_refused(self):
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [
            {
                "id": "p1", 
                "name": "peer-empty", 
                "connected": False, 
                "last_seen": make_time(35),
                "groups": []
            }
        ]
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        
        data = json.loads(out.getvalue())
        self.assertEqual(len(data["refused"]), 1)
        self.assertIn("no role group with mirror and no gear class group", data["refused"][0]["reason"])


    def test_multiple_roles_one_mirror_refused(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-OBSERVER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "p1", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1], self.nb.groups[3]]},
        ]
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        data = json.loads(out.getvalue())
        self.assertEqual(len(data["refused"]), 1)
        self.assertIn("role group(s) ZAGROS-OBSERVER have no PROBATION mirror", data["refused"][0]["reason"])

    def test_bad_lines_are_skipped_counted_and_reported(self):
        """ops/lifecycle/README.md: a bad line is skipped, never fatal, always
        counted. The report names the count in text and JSON; a partial
        record (required field missing) is a bad line too; blank lines are
        neither."""
        import io
        from contextlib import redirect_stdout, redirect_stderr
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p0", "name": "peer-zed-zagros-2026-09-01", "connected": True, "last_seen": make_time(0), "groups": []}]
        with open(self.hold_records, "w") as fh:
            fh.write("not json\n")
            fh.write("\n")
            fh.write(json.dumps({"peer_id": "p9", "v": 1}) + "\n")   # required fields missing
            fh.write(json.dumps([1, 2]) + "\n")                        # not an object
            fh.write(full_rec({"peer_id": "p1", "hold_group": "ZAGROS-PROBATION-DEVELOPER"}))
            fh.write("\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep(), 0)
        self.assertIn("HOLD RECORDS   1", out.getvalue())
        self.assertIn("3   bad line(s) skipped", out.getvalue())
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        hr = json.loads(out.getvalue())["hold_records"]
        self.assertEqual((hr["path"], hr["records"], hr["bad_lines_skipped"]),
                         (str(self.hold_records), 1, 3))
        self.assertEqual([o["peer_id"] for o in hr["orphans"]], ["p1"],
                         "p1 is not on the mesh in this fixture, so its record is an orphan")
        # A writer warns with the count and carries the bad lines through.
        err = io.StringIO()
        with redirect_stdout(io.StringIO()), redirect_stderr(err):
            self.assertIsNotNone(sweep.delete_hold_record("p1"))
        self.assertIn("WARNING 3 bad hold-record line(s)", err.getvalue())
        with open(self.hold_records) as fh:
            self.assertEqual([l for l in fh.read().splitlines() if l.strip()],
                             ["not json", json.dumps({"peer_id": "p9", "v": 1}), json.dumps([1, 2])])

    def test_report_without_record_file_says_nothing_about_hold_records(self):
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p0", "name": "peer-zed-zagros-2026-09-01", "connected": True, "last_seen": make_time(0), "groups": []}]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        self.assertIsNone(json.loads(out.getvalue())["hold_records"])

    def test_required_fields_match_schema(self):
        """The sweep's bad-line rule and the schema's required list must not drift
        (the console pins the same list from its side)."""
        schema = json.loads((HERE.parent / "lifecycle" / "schema" / "hold-record.v1.schema.json").read_text())
        self.assertEqual(sorted(schema["required"]), sorted(sweep.HOLD_RECORD_REQUIRED))

    def test_shared_bad_line_fixture_matches_the_console(self):
        """ops/lifecycle/fixtures/hold-records-bad-lines.jsonl is read by both
        readers' suites (this one and the console's meshstate tests) and both
        must count the same: 2 records, 10 bad lines, the blank line neither.
        The numbers are duplicated on purpose; move them in both places."""
        import io, shutil
        from contextlib import redirect_stdout
        shutil.copyfile(HERE.parent / "lifecycle" / "fixtures" / "hold-records-bad-lines.jsonl", self.hold_records)
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p0", "name": "peer-zed-zagros-2026-09-01", "connected": True, "last_seen": make_time(0), "groups": []}]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        hr = json.loads(out.getvalue())["hold_records"]
        self.assertEqual((hr["records"], hr["bad_lines_skipped"]), (2, 10))
        self.assertEqual(hr["releasing"], 1)

    def _stranded(self, record_prior, live_role="ZAGROS-DEVELOPER"):
        """A peer in its hold group AND a role group. With a record whose
        prior groups cover the role group it is a stranded hold (crash
        between add and remove); otherwise it needs hands."""
        self.nb.groups = [
            {"id": "g1", "name": live_role, "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-OBSERVER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        if record_prior is not None:
            with open(self.hold_records, "w") as fh:
                fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                                   "hold_group": "ZAGROS-PROBATION-DEVELOPER",
                                   "prior_groups": record_prior}) + "\n")

    def test_quarantine_apply_finishes_a_stranded_hold(self):
        """Crash after the add + record, before the removal: the next apply
        run completes the removal instead of leaving the peer dual-homed
        forever (#1040 review item 1)."""
        import io
        from contextlib import redirect_stdout
        self._stranded(record_prior=["ZAGROS-DEVELOPER"])
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("resumed", out.getvalue())
        self.assertIn("1 stranded hold(s) finished, 0 left in place by the group-choice rule, 0 failed", out.getvalue())
        put_calls = [c for c in self.nb.calls if c[0] == "PUT"]
        self.assertEqual([c[1] for c in put_calls], ["/api/groups/g1"])
        self.assertEqual(put_calls[0][2]["peers"], [])
        g2_peers = [x if isinstance(x, str) else x["id"] for x in self.nb.groups[1]["peers"]]
        self.assertIn("p1", g2_peers, "the hold group is kept; only the removal is finished")
        with open(self.hold_records) as fh:
            self.assertEqual(len([l for l in fh.read().splitlines() if l.strip()]), 1,
                             "the record stays: the peer is still held")

    def test_quarantine_apply_refuses_and_exits_1_when_the_record_does_not_cover_the_live_groups(self):
        """A record that never listed the live role group means an admin added
        it since the hold; the sweep does not remove what it never took,
        and the run exits 1 so the state cannot pass as a clean apply."""
        import io
        from contextlib import redirect_stdout
        self._stranded(record_prior=["ZAGROS-OBSERVER"])
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("REFUSED peer-bob-zagros-2026-09-01", out.getvalue())
        self.assertIn("needs hands", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_quarantine_apply_refuses_and_exits_1_without_a_record(self):
        """No record ⇒ security hold: a peer in a hold group and a role group
        with no record is never touched, and the apply run exits 1."""
        import io
        from contextlib import redirect_stdout
        self._stranded(record_prior=None)
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("REFUSED", out.getvalue())
        self.assertIn("0 peer(s) quarantined, 0 stranded hold(s) finished, 0 left in place by the group-choice rule, 1 failed", out.getvalue())
        self.assertIn("INCONSISTENT, needs hands", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_report_only_run_with_a_refusal_still_exits_0(self):
        """The hourly timer is report-only and its guard treats a non-zero
        exit as "a source could not be read" (roster and metrics left
        untouched). A refusal is world state the roster must publish, so
        the report exits 0 and carries it in `refused`."""
        import io
        from contextlib import redirect_stdout
        self._stranded(record_prior=None)
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        d = json.loads(out.getvalue())
        self.assertEqual([r["name"] for r in d["refused"]], ["peer-bob-zagros-2026-09-01"])
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_release_fallback_prints_the_ledger_row_it_would_act_on(self):
        """The refusal names the prior groups from the ledger row so the
        operator can judge the attestation before giving it."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p2"}]},
            {"id": "g4", "name": "ZAGROS-OBSERVER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p2", "name": "peer-ledger", "connected": False, "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
        ]
        with open(self.ledger, "w") as fh:
            fh.write("2026-09-01T00:00:00Z\tquarantine\tpeer-ledger\tp2\tZAGROS-OBSERVER\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p2", "--actor", "ops"), 1)
        self.assertIn("prior groups ZAGROS-OBSERVER", out.getvalue())
        self.assertIn("ZAGROS-QUARANTINED may be a security containment", out.getvalue())
        with open(self.hold_records) if self.hold_records.exists() else io.StringIO("") as fh:
            self.assertEqual(fh.read(), "")

    def test_release_second_run_after_a_record_release_reports_nothing_to_do(self):
        """Issue #1040 acceptance: hold, release, release again. The hold's
        ledger row outlives the record, so the repeat run used to fall into
        the QUARANTINED fallback and refuse with exit 1 for a peer sitting in
        its role group. Membership is checked before the attestation."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[2]]},
        ]
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        self.nb.peers[0]["groups"] = [self.nb.groups[1], self.nb.groups[2]]
        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops"), 0)
        self.nb.peers[0]["groups"] = [self.nb.groups[0], self.nb.groups[2]]
        self.nb.calls = []
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("nothing to release", out.getvalue())
        self.assertNotIn("security containment", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_release_removes_a_stale_record_for_a_peer_released_out_of_band(self):
        """Record says held, mesh says not (an admin moved the peer back by
        hand). The release run removes the stale line, says so, touches no
        group and exits 0."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": True,
             "last_seen": make_time(0), "groups": [self.nb.groups[0]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER",
                               "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("STALE RECORD peer-bob-zagros-2026-09-01", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")

    def test_quarantine_apply_group_choice_refusals_do_not_fail_the_run(self):
        """A two-role peer is refused by design (fail-closed until #1108). It
        is printed, left in its groups, and does NOT make the apply run exit
        1 — otherwise every apply on ZAGROS is red and the exit code says
        nothing about a genuinely stuck peer."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}, {"id": "p2"}]},
            {"id": "g2", "name": "ZAGROS-OBSERVER", "peers": [{"id": "p2"}]},
            {"id": "g3", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "g4", "name": "ZAGROS-PROBATION-OBSERVER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "one-role", "connected": False, "last_seen": make_time(15),
             "groups": [self.nb.groups[0]]},
            {"id": "p2", "name": "two-roles", "connected": False, "last_seen": make_time(15),
             "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("REFUSED two-roles", out.getvalue())
        self.assertIn("left in its groups", out.getvalue())
        self.assertIn("1 peer(s) quarantined, 0 stranded hold(s) finished, 1 left in place by the group-choice rule, 0 failed", out.getvalue())
        g1 = [x if isinstance(x, str) else x["id"] for x in self.nb.groups[0]["peers"]]
        g2 = [x if isinstance(x, str) else x["id"] for x in self.nb.groups[1]["peers"]]
        self.assertIn("p2", g1)
        self.assertIn("p2", g2)
        self.assertNotIn("p1", g1)

    def test_quarantine_apply_finishes_a_stranded_hold_on_a_connected_peer(self):
        """A stranded peer that came back online is dual-homed with full role
        access. It must not vanish from the candidate list because it is
        connected: the report lists it as inconsistent and the apply run
        finishes the removal."""
        import io
        from contextlib import redirect_stdout
        self._stranded(record_prior=["ZAGROS-DEVELOPER"])
        self.nb.peers[0]["connected"] = True
        self.nb.peers[0]["last_seen"] = make_time(0)
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        d = json.loads(out.getvalue())
        self.assertEqual([r["name"] for r in d["refused"]], ["peer-bob-zagros-2026-09-01"])
        self.assertTrue(d["refused"][0]["connected"])
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("finished a stranded hold", out.getvalue())
        self.assertIn("(peer is connected)", out.getvalue())
        g1 = [x if isinstance(x, str) else x["id"] for x in self.nb.groups[0]["peers"]]
        self.assertNotIn("p1", g1)

    def _ids(self, gid):
        return [x if isinstance(x, str) else x["id"]
                for x in [g for g in self.nb.groups if g["id"] == gid][0]["peers"]]

    def test_release_refuses_a_security_lane_record(self):
        """ADR 1091: a `security` record is never released by this tool, flag
        or no flag. No group is touched, the record stays, exit 1."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(3), "groups": [self.nb.groups[1]]},
        ]
        line = full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "lane": "security",
                         "hold_group": "ZAGROS-QUARANTINED", "prior_groups": ["ZAGROS-DEVELOPER"], "actor": "admin"})
        with open(self.hold_records, "w") as fh:
            fh.write(line + "\n")
        for extra in ((), ("--acknowledge-quarantined-is-dormancy",)):
            out = io.StringIO()
            with redirect_stdout(out):
                rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops", *extra)
            self.assertEqual(rc, 1)
            self.assertIn("SECURITY hold", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read().strip(), line)

    def test_release_refuses_a_dormancy_hold_with_a_containment_on_top(self):
        """A dormancy record, and the peer is ALSO in QUARANTINED (an admin
        contained it since). Restoring role groups would hand a contained
        peer its full reach; the release refuses and touches nothing."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[1], self.nb.groups[2]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("ZAGROS-QUARANTINED", out.getvalue())
        self.assertIn("live containment", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        self.assertNotIn("p1", self._ids("g1"))
        # The report lists the same peer as inconsistent: two hold groups.
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        d = json.loads(out.getvalue())
        self.assertIn("more than one hold group", d["refused"][0]["reason"])

    def test_stranded_gear_hold_is_listed_and_finished(self):
        """PROBATION-GEAR + SENSOR with a record listing SENSOR is the crash
        shape of every gear hold the sweep creates. It must be visible as
        inconsistent and finished by the next apply run, not silently
        skipped because SENSOR is not a canonical role group."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-SENSOR", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-GEAR", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-sensor-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(35), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-sensor-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-GEAR", "prior_groups": ["ZAGROS-SENSOR"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        d = json.loads(out.getvalue())
        self.assertEqual([r["name"] for r in d["refused"]], ["peer-sensor-zagros-2026-09-01"])
        self.assertIn("ZAGROS-SENSOR", d["refused"][0]["reason"])
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        self.assertIn("finished a stranded hold", out.getvalue())
        self.assertNotIn("p1", self._ids("g1"))
        self.assertIn("p1", self._ids("g2"))

    def test_release_refuses_an_ambiguous_name_and_never_deletes_another_peers_record(self):
        """Two live peers named `Mac`: pA held with a record, pB connected in
        DEVELOPER. Release by name refuses; release of pB by id finds no
        record (pA's is not a name match while pA is live) and pA's record
        survives."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "pB"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "pA"}]},
        ]
        self.nb.peers = [
            {"id": "pA", "name": "Mac", "connected": False, "last_seen": make_time(20), "groups": [self.nb.groups[1]]},
            {"id": "pB", "name": "Mac", "connected": True, "last_seen": make_time(0), "groups": [self.nb.groups[0]]},
        ]
        line = full_rec({"peer_id": "pA", "peer_name": "Mac", "hold_group": "ZAGROS-PROBATION-DEVELOPER",
                         "prior_groups": ["ZAGROS-DEVELOPER"]})
        with open(self.hold_records, "w") as fh:
            fh.write(line + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "Mac", "--actor", "ops"), 1)
        self.assertIn("share this name", out.getvalue())
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pB", "--actor", "ops"), 0)
        self.assertNotIn("STALE RECORD", out.getvalue())
        self.assertIn("nothing to release", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read().strip(), line)

    def test_quarantine_apply_refuses_when_groups_changed_since_the_report(self):
        """An admin adds QUARANTINED between the candidate list and the
        per-peer re-read. The hold must not record the containment as
        restore content and remove it: the rule is re-run on the fresh
        peer, the hold is refused, nothing is written."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
        ]
        real_api = self.nb.api
        def racing_api(path, method="GET", body=None, token=""):
            if method == "GET" and path == "/api/peers/p1":
                self.nb.peers[0]["groups"] = [self.nb.groups[0], self.nb.groups[2]]
                self.nb.groups[2]["peers"] = [{"id": "p1"}]
            return real_api(path, method, body, token)
        self.patch_api.stop()
        with patch("lifecycle_sweep.api", side_effect=racing_api):
            out = io.StringIO()
            with redirect_stdout(out):
                rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.patch_api.start()
        self.assertEqual(rc, 1)
        self.assertIn("groups changed since the report", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        self.assertFalse(self.hold_records.exists())

    def test_report_reads_hold_records_without_write_access(self):
        """The report-only run must not need write access to the record
        directory (a non-root hand run): it reads without the sidecar lock
        and exits 0 with the count."""
        import io
        import stat
        from contextlib import redirect_stdout
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "x", "connected": True, "last_seen": make_time(0),
                          "groups": [self.nb.groups[0]]}]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p9", "hold_group": "ZAGROS-PROBATION-DEVELOPER"}) + "\n")
        os.chmod(self.d, stat.S_IRUSR | stat.S_IXUSR)
        try:
            out = io.StringIO()
            with redirect_stdout(out):
                rc = self.run_sweep("--json")
        finally:
            os.chmod(self.d, stat.S_IRWXU)
        self.assertEqual(rc, 0)
        self.assertEqual(json.loads(out.getvalue())["hold_records"]["records"], 1)
        self.assertFalse((self.d / "hold_records.jsonl.lock").exists())

    def test_apply_with_json_is_refused(self):
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "x", "connected": True, "last_seen": make_time(0),
                          "groups": [self.nb.groups[0]]}]
        import io
        from contextlib import redirect_stderr
        err = io.StringIO()
        with redirect_stderr(err):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--json"), 2)
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--peer", "p1"), 2)
        self.assertIn("--json", err.getvalue())
        self.assertIn("--peer and --acknowledge-quarantined-is-dormancy only apply", err.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_group_put_carries_resources(self):
        """NetBird's updateGroup resets fields the body omits."""
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}], "resources": [{"id": "r1", "type": "host"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [], "resources": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
        ]
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        puts = {c[1]: c[2] for c in self.nb.calls if c[0] == "PUT"}
        self.assertEqual(puts["/api/groups/g1"]["resources"], [{"id": "r1", "type": "host"}])
        self.assertEqual(puts["/api/groups/g2"]["resources"], [])

    def test_apply_refusal_lines_carry_the_group_names(self):
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p2"}]},
            {"id": "g2", "name": "ZAGROS-OBSERVER", "peers": [{"id": "p2"}]},
            {"id": "g3", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
            {"id": "g4", "name": "ZAGROS-PROBATION-OBSERVER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p2", "name": "two-roles", "connected": False, "last_seen": make_time(15),
             "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        self.assertIn("groups=ZAGROS-DEVELOPER,ZAGROS-OBSERVER", out.getvalue())

    def test_concurrent_writer_changes_survive_the_sweeps_group_puts(self):
        """NetBird's group update is a whole-list replace. While the sweep is
        mid-run, the console holds ANOTHER peer (p8: into PROBATION, out of
        DEVELOPER). The sweep's PUTs must not revert that — they re-read
        each group immediately before writing."""
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}, {"id": "p8"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
            {"id": "p8", "name": "peer-eve-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
        ]
        real_api = self.nb.api
        state = {"done": False}
        def racing_api(path, method="GET", body=None, token=""):
            # The other writer acts after the sweep's initial group snapshot,
            # the moment the sweep re-reads p1 for its hold.
            if method == "GET" and path == "/api/peers/p1" and not state["done"]:
                state["done"] = True
                self.nb.groups[0]["peers"] = [{"id": "p1"}]
                self.nb.groups[1]["peers"] = [{"id": "p8"}]
                self.nb.peers[1]["groups"] = [self.nb.groups[1]]
            return real_api(path, method, body, token)
        self.patch_api.stop()
        try:
            with patch("lifecycle_sweep.api", side_effect=racing_api):
                rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--limit", "1")
        finally:
            self.patch_api.start()
        self.assertEqual(rc, 0)
        self.assertEqual(sorted(self._ids("g2")), ["p1", "p8"], "p8's hold survives")
        self.assertEqual(self._ids("g1"), [], "p8 is not put back into DEVELOPER")

    def test_ledger_fallback_never_restores_another_peers_row(self):
        """Two peers named `Mac`. pB (OBSERVER) was parked pre-change; pA
        (DEVELOPER,PLATFORM-ADMIN) was parked later and deleted since. The
        attested release of pB must restore pB's row, never pA's."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "pB"}]},
            {"id": "g4", "name": "ZAGROS-OBSERVER", "peers": []},
            {"id": "g5", "name": "ZAGROS-PLATFORM-ADMIN", "peers": []},
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "pB", "name": "Mac", "connected": False, "last_seen": make_time(20), "groups": [self.nb.groups[0]]},
        ]
        with open(self.ledger, "w") as fh:
            fh.write("2026-08-01T00:00:00Z\tquarantine\tMac\tpB\tZAGROS-OBSERVER\n")
            fh.write("2026-09-01T00:00:00Z\tquarantine\tMac\tpA\tZAGROS-DEVELOPER,ZAGROS-PLATFORM-ADMIN\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pB", "--actor", "ops"), 1)
        self.assertIn("peer id pB", out.getvalue())
        self.assertIn("prior groups ZAGROS-OBSERVER", out.getvalue())
        self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pB", "--actor", "ops",
                                        "--acknowledge-quarantined-is-dormancy"), 0)
        self.assertEqual(self._ids("g4"), ["pB"])
        self.assertEqual(self._ids("g5"), [])
        self.assertEqual(self._ids("g1"), [])
        # A second live `Mac` makes even a name-only row unusable.
        self.nb.groups[0]["peers"] = [{"id": "pC"}]
        self.nb.peers = [
            {"id": "pC", "name": "Mac", "connected": False, "last_seen": make_time(20), "groups": [self.nb.groups[0]]},
            {"id": "pD", "name": "Mac", "connected": True, "last_seen": make_time(0), "groups": [self.nb.groups[3]]},
        ]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pC", "--actor", "ops",
                                            "--acknowledge-quarantined-is-dormancy"), 0)
        self.assertIn("no hold record or ledger row", out.getvalue())

    def test_half_done_release_is_finished_by_release_and_not_undone_by_a_hold_run(self):
        """The hold-group PUT fails after the prior groups were restored. The
        record carries `phase: releasing`, so the next hold run refuses to
        "finish the hold" (which would re-remove the role group) and the
        next release finishes the release."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        real_api = self.nb.api
        def failing_api(path, method="GET", body=None, token=""):
            if method == "PUT" and path == "/api/groups/g2":
                raise Exception("502 Bad Gateway")
            return real_api(path, method, body, token)
        self.patch_api.stop()
        try:
            with patch("lifecycle_sweep.api", side_effect=failing_api):
                with redirect_stdout(io.StringIO()):
                    rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        finally:
            self.patch_api.start()
        self.assertEqual(rc, 1)
        self.assertEqual(self._ids("g1"), ["p1"])
        self.assertEqual(self._ids("g2"), ["p1"], "dual-homed: the half-done release shape")
        with open(self.hold_records) as fh:
            self.assertEqual(json.loads(fh.read())["phase"], "releasing")
        self.assertEqual(validator.validate_file(str(self.hold_records)), [])
        # The mesh now shows p1 in both groups.
        self.nb.peers[0]["groups"] = [self.nb.groups[0], self.nb.groups[1]]
        # A hold run must NOT treat this as a stranded hold.
        self.nb.calls = []
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("RELEASE is in progress", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        self.assertEqual(self._ids("g1"), ["p1"], "the restored role group is kept")
        # Release again finishes it.
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("resuming a release", out.getvalue())
        self.assertEqual(self._ids("g2"), [])
        self.assertEqual(self._ids("g1"), ["p1"])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")

    def test_orphan_record_is_reported_and_removed_by_release(self):
        """The peer was deleted in the NetBird UI; its record must not sit
        forever. The report counts it; release on its id removes it."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [{"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []}]
        self.nb.peers = [{"id": "pLIVE", "name": "peer-live", "connected": True, "last_seen": make_time(0), "groups": []}]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "pGONE", "peer_name": "peer-gone",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep(), 0)
        self.assertIn("1   record(s) whose peer no longer exists", out.getvalue())
        self.assertIn("pGONE", out.getvalue())
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pGONE", "--actor", "ops"), 0)
        self.assertIn("ORPHAN RECORD pGONE", out.getvalue())
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_orphan_release_refuses_mismatched_hold_timestamp_without_deleting(self):
        self.nb.groups = [{"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []}]
        self.nb.peers = [{"id": "pLIVE", "name": "peer-live", "connected": True,
                          "last_seen": make_time(0), "groups": []}]
        line = full_rec({"peer_id": "pGONE", "peer_name": "peer-gone",
                         "entered_at": "2026-10-20T10:11:12Z",
                         "hold_group": "ZAGROS-PROBATION-DEVELOPER",
                         "prior_groups": ["ZAGROS-DEVELOPER"]})
        with open(self.hold_records, "w") as fh:
            fh.write(line + "\n")
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "pGONE",
                                "--hold-entered-at", "2026-10-20T10:11:13Z", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("hold identity changed", out.getvalue())
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read().strip(), line)
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_resume_loop_never_acts_on_a_security_record(self):
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "lane": "security",
                               "hold_group": "ZAGROS-QUARANTINED", "prior_groups": ["ZAGROS-DEVELOPER"], "actor": "admin"}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 1)
        self.assertIn("security hold, not this tool's", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        self.assertEqual(self._ids("g1"), ["p1"])

    def test_lane_and_hold_group_disagreement_is_a_bad_line(self):
        """A hand-edited `dormancy` line pointing at QUARANTINED is skipped,
        so it cannot release a containment without the attestation."""
        import io
        from contextlib import redirect_stdout, redirect_stderr
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "lane": "dormancy",
                               "hold_group": "ZAGROS-QUARANTINED", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        self.assertEqual(json.loads(out.getvalue())["hold_records"]["bad_lines_skipped"], 1)
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("nothing to release", out.getvalue())
        self.assertNotIn("released", out.getvalue().replace("nothing to release", ""))
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_limit_is_one_budget_across_holds_and_resumes(self):
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}, {"id": "p2"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p2"}]},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
            {"id": "p2", "name": "peer-eve-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p2", "peer_name": "peer-eve-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--limit", "1"), 0)
        self.assertIn("p2", self._ids("g1"), "the stranded resume waits for the next run under --limit 1")
        self.assertNotIn("p1", self._ids("g1"))

    def test_peer_on_a_non_release_lane_is_refused_in_report_mode_too(self):
        import io
        from contextlib import redirect_stderr
        err = io.StringIO()
        with redirect_stderr(err):
            self.assertEqual(self.run_sweep("--lane", "quarantine", "--peer", "p1"), 2)
        self.assertIn("--peer and --acknowledge-quarantined-is-dormancy only apply", err.getvalue())
        self.assertEqual(self.nb.calls, [], "refused before any network call")

    def _stranded_behind_a_refusal(self):
        """p9 is a two-role refusal listed BEFORE stranded p2; p1 is a clean
        candidate."""
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}, {"id": "p2"}, {"id": "p9"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p2"}]},
            {"id": "g5", "name": "ZAGROS-OBSERVER", "peers": [{"id": "p9"}]},
            {"id": "g6", "name": "ZAGROS-PROBATION-OBSERVER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p9", "name": "two-roles", "connected": False, "last_seen": make_time(15),
             "groups": [self.nb.groups[0], self.nb.groups[2]]},
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
            {"id": "p2", "name": "peer-eve-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p2", "peer_name": "peer-eve-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")

    def test_limit_is_not_spent_on_group_choice_refusals(self):
        """A two-role refusal touches nothing, so it never consumes the
        budget and is always printed; the stranded peer behind it is reached
        on the second --limit 1 run (the first slot goes to the hold)."""
        import io
        from contextlib import redirect_stdout
        self._stranded_behind_a_refusal()
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--limit", "1"), 0)
        self.assertIn("REFUSED two-roles", out.getvalue())
        self.assertIn("NOT TOUCHED THIS RUN: 1 inconsistent peer(s)", out.getvalue())
        self.assertIn("1 left in place", out.getvalue())
        self.assertIn("p2", self._ids("g1"), "one slot, spent on p1's hold")
        self.nb.peers[1]["groups"] = [self.nb.groups[1]]   # the mesh after p1's hold
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--limit", "1"), 0)
        self.assertIn("finished a stranded hold", out.getvalue())
        self.assertNotIn("p2", self._ids("g1"))

    def test_limit_counts_a_failed_hold_as_an_attempt(self):
        """--limit 1: the hold of p1 fails; the stranded p2 must NOT be
        resumed in the same run — one peer touched means one attempted."""
        self._stranded_behind_a_refusal()
        real_api = self.nb.api
        def failing_api(path, method="GET", body=None, token=""):
            if method == "PUT" and path == "/api/groups/g2" and "p1" in (body or {}).get("peers", []):
                raise Exception("502 Bad Gateway")
            return real_api(path, method, body, token)
        self.patch_api.stop()
        try:
            with patch("lifecycle_sweep.api", side_effect=failing_api):
                rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--limit", "1")
        finally:
            self.patch_api.start()
        self.assertEqual(rc, 1)
        self.assertIn("p2", self._ids("g1"), "the failed hold spent the only slot")

    def test_release_re_reads_the_peer_before_deciding(self):
        """A containment added between the start-of-run peer list and the
        release lane must be seen: the release refuses instead of restoring
        role groups under QUARANTINED."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[1]]},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        real_api = self.nb.api
        def racing_api(path, method="GET", body=None, token=""):
            if method == "GET" and path == "/api/peers/p1":
                self.nb.groups[2]["peers"] = [{"id": "p1"}]
                self.nb.peers[0]["groups"] = [self.nb.groups[1], self.nb.groups[2]]
            return real_api(path, method, body, token)
        self.patch_api.stop()
        try:
            with patch("lifecycle_sweep.api", side_effect=racing_api):
                out = io.StringIO()
                with redirect_stdout(out):
                    rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        finally:
            self.patch_api.start()
        self.assertEqual(rc, 1)
        self.assertIn("live containment", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        self.assertEqual(self._ids("g1"), [])

    def test_hold_writes_the_record_before_the_add_so_a_lost_response_is_resumable(self):
        """The hold-group PUT lands server-side but the response is lost.
        With the record written first, the next run finishes the hold; with
        it written after, the peer had no record and needed hands forever."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": [self.nb.groups[0]]},
        ]
        real_api = self.nb.api
        def lossy_api(path, method="GET", body=None, token=""):
            r = real_api(path, method, body, token)
            if method == "PUT" and path == "/api/groups/g2":
                raise Exception("timed out (the PUT was applied)")
            return r
        self.patch_api.stop()
        try:
            with patch("lifecycle_sweep.api", side_effect=lossy_api):
                with redirect_stdout(io.StringIO()):
                    rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        finally:
            self.patch_api.start()
        self.assertEqual(rc, 1)
        self.assertEqual(self._ids("g2"), ["p1"])
        self.assertEqual(self._ids("g1"), ["p1"], "dual-homed")
        self.assertEqual(validator.validate_file(str(self.hold_records)), [])
        self.nb.peers[0]["groups"] = [self.nb.groups[0], self.nb.groups[1]]
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        self.assertIn("finished a stranded hold", out.getvalue())
        self.assertEqual(self._ids("g1"), [])

    def test_release_finished_restores_prior_groups_before_deleting_a_releasing_record(self):
        """The release died before its first restore; an admin emptied the
        hold group by hand. The next release must put the prior groups back
        before the only copy of the restore content is deleted."""
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": []},
            {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []},
        ]
        self.nb.peers = [
            {"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
             "last_seen": make_time(15), "groups": []},
        ]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "phase": "releasing",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops"), 0)
        self.assertIn("RELEASE FINISHED", out.getvalue())
        self.assertEqual(self._ids("g1"), ["p1"])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read(), "")

    def test_unknown_phase_is_a_bad_line_like_the_validator_says(self):
        import io
        from contextlib import redirect_stdout
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "x", "connected": True, "last_seen": make_time(0), "groups": []}]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "phase": "bogus"}) + "\n")
        self.assertNotEqual(validator.validate_file(str(self.hold_records)), [])
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        self.assertEqual(json.loads(out.getvalue())["hold_records"]["bad_lines_skipped"], 1)

    def test_release_on_a_stranded_hold_points_at_the_quarantine_lane(self):
        import io
        from contextlib import redirect_stdout
        self._stranded(record_prior=["ZAGROS-DEVELOPER"])
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops"), 1)
        self.assertIn("--apply --lane quarantine", out.getvalue())

    def test_revoke_skips_a_peer_under_a_security_hold(self):
        self.nb.groups = [{"id": "g3", "name": "ZAGROS-QUARANTINED", "peers": [{"id": "p1"}]}]
        self.nb.peers = [{"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
                          "last_seen": make_time(130), "groups": [self.nb.groups[0]]}]
        line = full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01", "lane": "security",
                         "hold_group": "ZAGROS-QUARANTINED", "prior_groups": ["ZAGROS-DEVELOPER"], "actor": "admin"})
        with open(self.hold_records, "w") as fh:
            fh.write(line + "\n")
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "revoke", "--actor", "ops"), 0)
        self.assertIn("skip (SECURITY hold, ADR 1091)", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "DELETE"], [])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read().strip(), line)

    def test_orphan_exit_refuses_a_security_record(self):
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "pLIVE", "name": "x", "connected": True, "last_seen": make_time(0), "groups": []}]
        line = full_rec({"peer_id": "pGONE", "peer_name": "peer-gone", "lane": "security",
                         "hold_group": "ZAGROS-QUARANTINED", "prior_groups": [], "actor": "admin"})
        with open(self.hold_records, "w") as fh:
            fh.write(line + "\n")
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "pGONE", "--actor", "ops"), 1)
        self.assertIn("SECURITY hold", out.getvalue())
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read().strip(), line)

    def test_release_re_read_failure_is_reported_not_a_traceback(self):
        self.nb.groups = [{"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]}]
        self.nb.peers = [{"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
                          "last_seen": make_time(15), "groups": [self.nb.groups[0]]}]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": []}) + "\n")
        real_api = self.nb.api
        def gone_api(path, method="GET", body=None, token=""):
            if method == "GET" and path == "/api/peers/p1":
                raise Exception("404 peer not found")
            return real_api(path, method, body, token)
        import io
        from contextlib import redirect_stdout
        self.patch_api.stop()
        try:
            with patch("lifecycle_sweep.api", side_effect=gone_api):
                out = io.StringIO()
                with redirect_stdout(out):
                    rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        finally:
            self.patch_api.start()
        self.assertEqual(rc, 1)
        self.assertIn("could not re-read the peer", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])

    def test_report_names_a_stale_record_for_a_reconnected_peer(self):
        """A record exists but the live peer is not in its hold group and is
        connected: the candidate list cannot see it, the summary must."""
        self.nb.groups = [{"id": "g1", "name": "ZAGROS-DEVELOPER", "peers": [{"id": "p1"}]},
                          {"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": True,
                          "last_seen": make_time(0), "groups": [self.nb.groups[0]]}]
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                               "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-DEVELOPER"]}) + "\n")
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        hr = json.loads(out.getvalue())["hold_records"]
        self.assertEqual([o["peer_id"] for o in hr["stale"]], ["p1"])
        self.assertEqual(hr["orphans"], [])

    def test_release_refuses_when_a_prior_group_no_longer_exists(self):
        self.nb.groups = [{"id": "g2", "name": "ZAGROS-PROBATION-DEVELOPER", "peers": [{"id": "p1"}]}]
        self.nb.peers = [{"id": "p1", "name": "peer-bob-zagros-2026-09-01", "connected": False,
                          "last_seen": make_time(15), "groups": [self.nb.groups[0]]}]
        line = full_rec({"peer_id": "p1", "peer_name": "peer-bob-zagros-2026-09-01",
                         "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": ["ZAGROS-GONE"]})
        with open(self.hold_records, "w") as fh:
            fh.write(line + "\n")
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops"), 1)
        self.assertIn("ZAGROS-GONE", out.getvalue())
        self.assertEqual([c for c in self.nb.calls if c[0] == "PUT"], [])
        with open(self.hold_records) as fh:
            self.assertEqual(fh.read().strip(), line, "record untouched, not even marked releasing")

    def test_negative_limit_and_stray_attestation_flag_are_refused_before_network(self):
        import io
        from contextlib import redirect_stderr
        err = io.StringIO()
        with redirect_stderr(err):
            self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--limit", "-1"), 2)
            self.assertEqual(self.run_sweep("--lane", "quarantine", "--acknowledge-quarantined-is-dormancy"), 2)
        self.assertIn("--limit must be", err.getvalue())
        self.assertEqual(self.nb.calls, [])

    def test_a_corrupt_byte_is_a_bad_line_not_a_report_crash(self):
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": "p9", "hold_group": "ZAGROS-PROBATION-DEVELOPER"}) + "\n")
        with open(self.hold_records, "ab") as fh:
            fh.write(b'{"v": 1, "peer_id": "\xff\xfe"}\n')
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "x", "connected": True, "last_seen": make_time(0), "groups": []}]
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        hr = json.loads(out.getvalue())["hold_records"]
        self.assertEqual((hr["records"], hr["bad_lines_skipped"]), (1, 1))

    def test_type_invalid_fields_are_bad_lines_not_a_report_crash(self):
        with open(self.hold_records, "w") as fh:
            fh.write(full_rec({"peer_id": ["p1"], "hold_group": "ZAGROS-PROBATION-DEVELOPER"}) + "\n")
            fh.write(full_rec({"peer_id": "p2", "hold_group": ["ZAGROS-PROBATION-DEVELOPER"]}) + "\n")
            fh.write(full_rec({"peer_id": "p3", "hold_group": "ZAGROS-PROBATION-DEVELOPER", "prior_groups": "ZAGROS-DEVELOPER"}) + "\n")
            fh.write(full_rec({"peer_id": "p4", "hold_group": "ZAGROS-PROBATION-DEVELOPER"}) + "\n")
        self.nb.groups = [{"id": "gAll", "name": "All", "peers": []}]
        self.nb.peers = [{"id": "p1", "name": "x", "connected": True, "last_seen": make_time(0), "groups": []}]
        import io
        from contextlib import redirect_stdout
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(self.run_sweep("--json"), 0)
        hr = json.loads(out.getvalue())["hold_records"]
        self.assertEqual((hr["records"], hr["bad_lines_skipped"]), (1, 3))

    def test_record_handle_is_null_when_the_grammar_is_ambiguous(self):
        self.assertEqual(sweep.record_handle("peer-user290x-zagros-2026-09-01"), "user290x")
        self.assertEqual(sweep.record_handle("peer-jane-doe-zagros-2026-09-01-laptop"), None)
        self.assertEqual(sweep.record_handle("peer-dave-2-usareur-af-zagros-2026-09-01"), None)
        self.assertEqual(sweep.record_handle("Mac"), None)



class GovernedScopeTest(unittest.TestCase):
    """mesh-change-control.md §4.5: on a governed scope the hold is an entry."""

    def setUp(self):
        self.nb = MockNetBird()
        self.patch_api = patch("lifecycle_sweep.api", side_effect=self.nb.api)
        self.patch_api.start()
        self.temp_dir = tempfile.TemporaryDirectory()
        self.d = Path(self.temp_dir.name)
        # The scope DIRECTORY the warrant-membership role installs at
        # /etc/goat/warrant/membership/<scope>/.
        self.sdir = self.d / "zagros-dev"
        self.sdir.mkdir()
        (self.sdir / "_scope.yaml").write_text("version: 1\nscope: zagros-dev\nconverge: enforce\n")
        (self.sdir / "_baseline.generated.yaml").write_text(
            "version: 1\nscope: zagros-dev\ngenerated_from: test\nidentity_hashes: {}\npeople: {}\n")
        self.ledger = self.d / "ledger.log"
        self.hold_records = self.d / "hold_records.jsonl"
        self.env_patcher = patch.dict(os.environ, {
            "NB_TOKEN": "fake-token",
            "LIFECYCLE_LEDGER": str(self.ledger),
            "LIFECYCLE_HOLD_RECORDS": "",
            "LIFECYCLE_MEMBERSHIP_DIR": str(self.sdir),
            "GOATNET": "zagros-dev",
        })
        self.env_patcher.start()
        sweep.SITE = "zagros-dev"
        sweep.HOLD_RECORDS = ""
        sweep.LEDGER = str(self.ledger)
        self.nb.groups = [
            {"id": "g1", "name": "ZAGROS-DEV-DEVELOPER", "peers": [{"id": "p1"}]},
            {"id": "g2", "name": "ZAGROS-DEV-PROBATION-DEVELOPER", "peers": []},
            {"id": "gAll", "name": "All", "peers": [{"id": "p1"}]},
        ]
        self.nb.peers = [{
            "id": "p1", "name": "peer-bob-zagros-dev-2026-09-01", "connected": False,
            "last_seen": make_time(15), "groups": [self.nb.groups[0], self.nb.groups[2]],
        }]

    def tearDown(self):
        self.env_patcher.stop()
        self.patch_api.stop()
        self.temp_dir.cleanup()
        sweep.SITE = "zagros"

    def run_sweep(self, *args):
        with patch("sys.argv", ["lifecycle-sweep.py"] + list(args)):
            return sweep.main()

    def test_detects_governed_dir(self):
        self.assertEqual(sweep.membership_file(), str(self.sdir))

    def test_report_scope_is_not_governed(self):
        (self.sdir / "_scope.yaml").write_text("version: 1\nscope: zagros-dev\nconverge: report\n")
        self.assertIsNone(sweep.membership_file())

    def test_missing_scope_file_fails_toward_governed(self):
        (self.sdir / "_scope.yaml").unlink()
        self.assertEqual(sweep.membership_file(), str(self.sdir))

    def test_host_lookup_is_the_scope_directory(self):
        os.environ.pop("LIFECYCLE_MEMBERSHIP_DIR", None)
        with patch.object(sweep, "MEMBERSHIP_ETC_DIR", str(self.d)), \
                patch.object(sweep, "MEMBERSHIP_REPO_DIR", str(self.d / "none")):
            self.assertEqual(sweep.membership_file("zagros-dev"), str(self.sdir))
            self.assertIsNone(sweep.membership_file("zagros"))
            # A host still carrying the retired single file stays governed.
            (self.d / "zagros.yaml").write_text("version: 1\nscope: zagros\nconverge: enforce\nentries: []\n")
            self.assertEqual(sweep.membership_file("zagros"), str(self.d / "zagros.yaml"))

    def test_quarantine_writes_nothing_live_and_emits_entry(self):
        out = self.d / "holds.yaml"
        rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--entries-out", str(out))
        self.assertEqual(rc, 0)
        self.assertEqual([c for c in self.nb.calls if c[0] != "GET"], [])
        self.assertFalse(self.hold_records.exists())
        text = out.read_text()
        self.assertIn("- peer: peer-bob-zagros-dev-2026-09-01", text)
        self.assertIn("exact: [ZAGROS-DEV-PROBATION-DEVELOPER]", text)

        # The emitted entry drops into the scope's holds.yaml and validates,
        # against the test's own taxonomy, so the case does not depend on which
        # goatnets the repository's ops/ansible/netbird_acl_taxonomy.yml lists.
        sys.path.insert(0, str(HERE.parent / "warrant"))
        import membership as mem
        taxonomy = self.d / "netbird_acl_taxonomy.yml"
        taxonomy.write_text(
            "netbird_acl_taxonomy:\n"
            "  zagros-dev:\n"
            "    site_prefix: ZAGROS-DEV\n"
            "    groups:\n"
            "    - suffix: DEVELOPER\n"
            "    - suffix: PROBATION-DEVELOPER\n")
        (self.sdir / "holds.yaml").write_text("version: 1\nentries:\n" + text)
        self.assertEqual(mem.check_scope(self.sdir, scopes=mem.taxonomy_scopes(taxonomy)), [])
        self.assertEqual(len(mem.load_scope(self.sdir)["entries"]), 1)

    def test_hold_for_a_declared_peer_replaces_its_entry(self):
        """One entry per peer: a peer already in people.yaml is not emitted as a
        new holds.yaml row; the output says which fragment to edit."""
        import io
        from contextlib import redirect_stdout
        (self.sdir / "people.yaml").write_text(
            "version: 1\nentries:\n  - peer: peer-bob-zagros-dev-2026-09-01\n"
            "    grant: [ZAGROS-DEV-MONITOR]\n    owner: o\n    reason: r\n")
        out = self.d / "holds.yaml"
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops", "--entries-out", str(out))
        self.assertEqual(rc, 0)
        self.assertEqual(out.read_text(), "")
        self.assertIn("REPLACES the existing entry for peer-bob-zagros-dev-2026-09-01", buf.getvalue())
        self.assertIn("zagros-dev/people.yaml", buf.getvalue())

    def test_quarantine_to_stdout(self):
        import io
        from contextlib import redirect_stdout
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops")
        self.assertEqual(rc, 0)
        self.assertIn("exact: [ZAGROS-DEV-PROBATION-DEVELOPER]", buf.getvalue())
        self.assertIn("nothing was written live", buf.getvalue())

    def test_release_refused(self):
        rc = self.run_sweep("--apply", "--lane", "release", "--peer", "p1", "--actor", "ops")
        self.assertEqual(rc, 2)
        self.assertEqual([c for c in self.nb.calls if c[0] != "GET"], [])

    def test_ungoverned_still_writes_live(self):
        os.environ["LIFECYCLE_MEMBERSHIP_DIR"] = str(self.d / "absent")
        os.environ["LIFECYCLE_HOLD_RECORDS"] = str(self.hold_records)
        sweep.HOLD_RECORDS = str(self.hold_records)
        self.assertEqual(self.run_sweep("--apply", "--lane", "quarantine", "--actor", "ops"), 0)
        self.assertTrue(any(c[0] == "PUT" for c in self.nb.calls))

if __name__ == "__main__":
    unittest.main(verbosity=2)
