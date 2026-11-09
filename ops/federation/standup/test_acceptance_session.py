"""The bilateral session: a manifest both sides import, bundles exchanged by file,
and the partner's reported results kept apart from our own."""
import contextlib
import datetime
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


def _load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + ".py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ses = _load("acceptance_session")
fakefabric = _load("acceptance_fakefabric")
NOW = datetime.datetime(2026, 11, 9, 12, 0, 0, tzinfo=datetime.timezone.utc)


def at(minutes):
    return NOW + datetime.timedelta(minutes=minutes)


class Session(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="acceptance-session-")
        self.ag = os.path.join(self.d, "agreement.json")
        with open(self.ag, "w") as f:
            json.dump(fakefabric.agreement(), f)
        self.manifest = os.path.join(self.d, "session.json")
        self.m = ses.new_manifest(self.ag, "xaa", hours=2, now=NOW)
        with open(self.manifest, "w") as f:
            json.dump(self.m, f)
        self.evidence = os.path.join(self.d, "evidence")

    def peer_bundle(self, **over):
        """A bundle as xab's runner writes it in this session."""
        b = {
            "schema": "goat-federation-acceptance/v1",
            "run": {"id": over.get("run_id", "acc-xab-1"), "started_at": over.get("start", "2026-11-09T12:05:00Z"),
                    "ended_at": over.get("end", "2026-11-09T12:06:00Z"), "us": "xab", "them": "xaa",
                    "observed_by": over.get("observed_by", "xab")},
            "agreement": {"sha256": over.get("agreement", self.m["agreement_sha256"])},
            "session": {"id": over.get("session", self.m["session_id"]),
                        "manifest_sha256": over.get("manifest_sha", ses.sha256_file(self.manifest)),
                        "challenge": over.get("challenge", self.m["challenges"]["xab"])},
            "checks": over.get("checks", [
                {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS",
                 "observations": {"received": over.get("received", "xaa-%s-all" % self.m["challenges"]["xaa"])}},
                {"name": "republisher-boundary/inbound", "required": True, "verdict": "PASS", "observations": {}},
            ]),
        }
        p = os.path.join(self.d, "peer-%s.json" % b["run"]["id"])
        with open(p, "w") as f:
            json.dump(b, f)
        return p

    def test_the_manifest_binds_what_the_proposal_lists(self):
        for k in ("session_id", "agreement_sha256", "policy", "participants", "not_before", "expires_at",
                  "challenges", "limits"):
            self.assertIn(k, self.m)
        self.assertEqual(sorted(self.m["challenges"]), ["xaa", "xab"])
        self.assertNotEqual(self.m["challenges"]["xaa"], self.m["challenges"]["xab"])
        self.assertEqual(ses.validate(self.m, self.ag, "xab", now=at(1)), "xaa")

    def test_validate_refuses_another_agreement_a_stranger_and_the_wrong_time(self):
        other = os.path.join(self.d, "other.json")
        with open(other, "w") as f:
            json.dump(fakefabric.agreement(them="xac"), f)
        for args, now in (((self.m, other, "xaa"), at(1)), ((self.m, self.ag, "xac"), at(1)),
                          ((self.m, self.ag, "xaa"), at(-1)), ((self.m, self.ag, "xaa"), at(121))):
            with self.subTest(args=args[1:], now=now), self.assertRaises(ses.Refused):
                ses.validate(*args, now=now)

    def test_a_good_bundle_is_imported_as_peer_reported(self):
        them, path = ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(), now=at(10))
        self.assertEqual(them, "xab")
        stored = json.load(open(path))
        self.assertEqual(stored["imported"]["peer_reported"], True)
        self.assertEqual(stored["imported"]["authenticated"], False)
        self.assertTrue(path.startswith(os.path.join(self.evidence, "peer", "xab")))

    def test_mismatched_bundles_are_refused(self):
        cases = {
            "our own bundle": dict(observed_by="xaa"),
            "another session": dict(session="ses-other"),
            "another manifest": dict(manifest_sha="sha256:" + "0" * 64),
            "another agreement": dict(agreement="sha256:" + "1" * 64),
            "another challenge": dict(challenge="00"),
            "a run outside the window": dict(start="2026-11-09T11:00:00Z"),
            "an inbound delivery without our challenge": dict(received="xaa-stale-all"),
        }
        for why, over in cases.items():
            with self.subTest(why=why), self.assertRaises(ses.Refused):
                ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(**over), now=at(10))

    def test_a_replayed_or_late_bundle_is_refused(self):
        p = self.peer_bundle()
        ses.import_bundle(self.manifest, "xaa", self.evidence, p, now=at(10))
        with self.assertRaises(ses.Refused):
            ses.import_bundle(self.manifest, "xaa", self.evidence, p, now=at(11))
        with self.assertRaises(ses.Refused):
            ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(run_id="acc-xab-2"), now=at(121))

    def write_local(self, checks, session=None):
        os.makedirs(self.evidence, exist_ok=True)
        with open(os.path.join(self.evidence, "result.json"), "w") as f:
            json.dump({"schema": "goat-federation-acceptance/v1", "run": {"id": "acc-xaa-1"},
                       "session": {"id": session or self.m["session_id"]}, "checks": checks}, f)

    def test_combine_keeps_local_and_peer_reported_apart_and_needs_both(self):
        local = [
            {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS", "observations": {}},
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
             "observations": {"observed_by_peer": "delivery/inbound/release/all"}},
        ]
        self.write_local(local)
        out = ses.combine(self.manifest, "xaa", self.evidence)
        self.assertEqual(out["verdict"], "INCOMPLETE")
        self.assertIn("no bundle from xab imported for this session", out["open"])
        ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(), now=utc_now_inside(self.m))
        out = ses.combine(self.manifest, "xaa", self.evidence)
        self.assertEqual(out["verdict"], "PASS", out)
        self.assertFalse(out["authenticated"])
        self.assertIn("operator review required", out["gate"])
        row = [r for r in out["rows"] if r["name"] == "delivery/outbound/release/all"][0]
        self.assertEqual((row["local"], row["status"]), ("BLOCKED", "PASS (peer-reported)"))
        self.assertTrue([r for r in out["rows"] if r["name"] == "xab: delivery/inbound/release/all"])
        self.assertTrue(os.path.exists(os.path.join(self.evidence, "bilateral", self.m["session_id"] + ".md")))

    def test_combine_fails_on_a_peer_reported_failure_and_refuses_another_session(self):
        self.write_local([{"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
                           "observations": {"observed_by_peer": "delivery/inbound/release/all"}}])
        failing = self.peer_bundle(checks=[{"name": "delivery/inbound/release/all", "required": True,
                                            "verdict": "FAIL", "observations": {"received": ""}}])
        ses.import_bundle(self.manifest, "xaa", self.evidence, failing, now=utc_now_inside(self.m))
        self.assertEqual(ses.combine(self.manifest, "xaa", self.evidence)["verdict"], "FAIL")
        self.write_local([], session="ses-other")
        with self.assertRaises(ses.Refused):
            ses.combine(self.manifest, "xaa", self.evidence)

    def test_the_cli_writes_a_manifest_once(self):
        out = os.path.join(self.d, "new.json")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(ses.main(["new", "--agreement", self.ag, "--us", "xaa", "--out", out]), 0)
            self.assertEqual(ses.main(["new", "--agreement", self.ag, "--us", "xaa", "--out", out]), 2)
            self.assertEqual(ses.main(["validate", "--manifest", out, "--agreement", self.ag, "--us", "xab"]), 0)


def utc_now_inside(m):
    """`combine` reads real time nowhere; imports in these tests need a time inside the window."""
    return ses.parse_ts(m["not_before"]) + datetime.timedelta(minutes=10)


if __name__ == "__main__":
    unittest.main()
