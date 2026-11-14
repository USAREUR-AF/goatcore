"""The bilateral session: a manifest both sides import, bundles exchanged by file,
and the partner's reported results kept apart from our own."""
import contextlib
import datetime
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
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


class Fixture(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix="acceptance-session-")
        self.addCleanup(shutil.rmtree, self.d, True)
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
        for c in b["checks"]:
            if over.get("raw"):
                break
            fill(c)
        p = os.path.join(self.d, "peer-%s.json" % b["run"]["id"].replace("/", "_"))
        with open(p, "w") as f:
            json.dump(b, f)
        return p


class Session(Fixture):
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
        checks = [fill(c) for c in checks]
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
        peer = self.peer_bundle(checks=[
            {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS",
             "observations": {"received": "xaa-%s-all" % self.m["challenges"]["xaa"]}},
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
             "observations": {"observed_by_peer": "delivery/inbound/release/all"}}])
        ses.import_bundle(self.manifest, "xaa", self.evidence, peer, now=utc_now_inside(self.m))
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

    def test_a_manifest_with_shell_or_path_characters_is_refused(self):
        for field, value in (("session_id", "ses-x';touch${IFS}/tmp/p;'"), ("session_id", "../../x"),
                             ("challenges", {"xaa": "x';id;'", "xab": "0" * 32})):
            with self.subTest(field=field, value=value), self.assertRaises(ses.Refused):
                ses.validate(dict(self.m, **{field: value}), self.ag, "xaa", now=at(1))

    def test_combine_resolves_the_partners_rows_that_we_observed(self):
        self.write_local([
            {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS", "observations": {}},
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
             "observations": {"observed_by_peer": "delivery/inbound/release/all"}}])
        peer = self.peer_bundle(checks=[
            {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS",
             "observations": {"received": "xaa-%s-all" % self.m["challenges"]["xaa"]}},
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
             "observations": {"observed_by_peer": "delivery/inbound/release/all"}}])
        ses.import_bundle(self.manifest, "xaa", self.evidence, peer, now=utc_now_inside(self.m))
        out = ses.combine(self.manifest, "xaa", self.evidence)
        row = [r for r in out["rows"] if r["name"] == "xab: delivery/outbound/release/all"][0]
        self.assertEqual(row["status"], "PASS (local)")
        self.assertEqual(out["verdict"], "PASS", out)

    def test_a_manifest_must_name_us_among_two_well_formed_participants(self):
        for why, parts in (("a stranger", ["xab", "xac"]), ("three", ["xaa", "xab", "xac"]),
                           ("a malformed id", ["xaa", "../x"])):
            with self.subTest(why=why):
                m = dict(self.m, participants=[{"sovereign_id": i} for i in parts],
                         challenges={i: "0" * 32 for i in parts})
                with open(self.manifest, "w") as f:
                    json.dump(m, f)
                with self.assertRaises(ses.Refused):
                    ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(), now=at(10))

    def test_run_ids_have_one_shape_and_a_bundle_counts_once_by_its_digest(self):
        with self.assertRaises(ses.Refused):
            ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(run_id="../../x"), now=at(10))
        p = self.peer_bundle()
        ses.import_bundle(self.manifest, "xaa", self.evidence, p, now=at(10))
        copy = os.path.join(self.d, "copy.json")
        shutil.copy(p, copy)
        with self.assertRaises(ses.Refused):
            ses.import_bundle(self.manifest, "xaa", self.evidence, copy, now=at(11))
        index = json.load(open(os.path.join(self.evidence, "peer", "xab", "imported.json")))
        self.assertEqual(list(index), [ses.sha256_file(p)])

    def test_combine_requires_the_mirror_of_every_row_of_ours_and_ignores_their_required(self):
        self.write_local([
            {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS", "observations": {}},
            {"name": "refusal/control-allowed", "required": True, "verdict": "PASS", "observations": {}},
            {"name": "publish/outbound/release/all", "required": True, "verdict": "PASS", "observations": {}}])
        # Their bundle holds the mirror of our delivery row, but marks its refusal row
        # optional and SKIPPED: still required here, so the scope stays open.
        peer = self.peer_bundle(checks=[
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
             "observations": {"observed_by_peer": "delivery/inbound/release/all"}},
            {"name": "refusal/control-allowed", "required": False, "verdict": "SKIP", "observations": {}}])
        ses.import_bundle(self.manifest, "xaa", self.evidence, peer, now=utc_now_inside(self.m))
        out = ses.combine(self.manifest, "xaa", self.evidence)
        self.assertEqual(out["verdict"], "INCOMPLETE", out)
        self.assertIn("xab: refusal/control-allowed", out["open"])
        self.assertNotIn("xab: publish/inbound/release/all", [r["name"] for r in out["rows"]])

    def test_a_missing_or_malformed_peer_row_is_not_a_pass(self):
        self.write_local([{"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS",
                           "observations": {}}])
        peer = self.peer_bundle(raw=True, checks=[
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "PASS", "direction": "sideways",
             "reason": "", "started_at": "2026-11-09T12:05:00Z", "observations": {}}])
        ses.import_bundle(self.manifest, "xaa", self.evidence, peer, now=utc_now_inside(self.m))
        out = ses.combine(self.manifest, "xaa", self.evidence)
        self.assertEqual(out["verdict"], "FAIL", out)

    def test_the_cli_writes_a_manifest_once(self):
        out = os.path.join(self.d, "new.json")
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(ses.main(["new", "--agreement", self.ag, "--us", "xaa", "--out", out]), 0)
            self.assertEqual(ses.main(["new", "--agreement", self.ag, "--us", "xaa", "--out", out]), 2)
            self.assertEqual(ses.main(["validate", "--manifest", out, "--agreement", self.ag, "--us", "xab"]), 0)


def fill(c):
    """The fields acceptance_evidence.py writes on every row."""
    n = c["name"]
    c.setdefault("direction", "inbound" if "/inbound" in n else "outbound" if "/outbound" in n else "both")
    c.setdefault("reason", "seen")
    c.setdefault("started_at", "2026-11-09T12:05:00Z")
    c.setdefault("ended_at", "2026-11-09T12:05:30Z")
    return c
PARTNER = """sovereign_id: xab
display_name: "Synthetic partner"
level: exchanging
partner_cn: fed.xab.01
ingress:
  - "from/xab/**"
egress:
  - "from/xaa/all/**"
rationale: >-
  Synthetic declaration for the acceptance publishing tests. Not a peering.
"""


class Publish(Fixture):
    """`exchanging` is published only as a reviewable change, after both sides passed."""

    def ready(self, cleanup=True, peer_cleanup=True, level="exchanging"):
        local = [
            {"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS", "observations": {}},
            {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
             "observations": {"observed_by_peer": "delivery/inbound/release/all"}},
        ]
        if cleanup:
            local.append({"name": "cleanup/probes", "required": True, "verdict": "PASS", "observations": {}})
        os.makedirs(self.evidence, exist_ok=True)
        with open(os.path.join(self.evidence, "result.json"), "w") as f:
            json.dump({"schema": "goat-federation-acceptance/v1", "run": {"id": "acc-xaa-1"},
                       "agreement": {"path": self.ag}, "session": {"id": self.m["session_id"]}, "checks": local}, f)
        checks = [{"name": "delivery/inbound/release/all", "required": True, "verdict": "PASS",
                   "observations": {"received": "xaa-%s-all" % self.m["challenges"]["xaa"]}},
                  {"name": "delivery/outbound/release/all", "required": True, "verdict": "BLOCKED",
                   "observations": {"observed_by_peer": "delivery/inbound/release/all"}}]
        if peer_cleanup:
            checks.append({"name": "cleanup/probes", "required": True, "verdict": "PASS", "observations": {}})
        ses.import_bundle(self.manifest, "xaa", self.evidence, self.peer_bundle(checks=checks),
                          now=utc_now_inside(self.m))
        self.partner = os.path.join(self.d, "xab.yaml")
        with open(self.partner, "w") as f:
            f.write(PARTNER.replace("level: exchanging", "level: " + level))
        self.out = os.path.join(self.d, "proposed")

    def propose(self, approvals=("xaa=Operator A", "xab=Operator B")):
        return ses.propose(self.manifest, "xaa", self.evidence, self.partner, list(approvals), self.out,
                           now=utc_now_inside(self.m))

    def test_a_passing_bilateral_result_becomes_a_proposed_change_not_an_edit(self):
        self.ready()
        before = open(self.partner).read()
        proposed, diff = self.propose()
        self.assertEqual(open(self.partner).read(), before, "the deployment's file was edited in place")
        text = open(proposed).read()
        self.assertIn("status: exchanging", text)
        self.assertIn('session: "%s"' % self.m["session_id"], text)
        self.assertIn("level: exchanging", text)
        self.assertIn("+acceptance:", diff)
        self.assertTrue(os.path.exists(proposed + ".diff"))
        import yaml
        d = yaml.safe_load(text)
        self.assertEqual(d["acceptance"]["approved_by"], {"xaa": "Operator A", "xab": "Operator B"})
        self.assertEqual(d["acceptance"]["expires"], "2026-12-01")
        # A second session replaces the block instead of adding another.
        with open(self.partner, "w") as f:
            f.write(text)
        proposed, _ = self.propose()
        self.assertEqual(open(proposed).read().count("acceptance:"), 1)

    def test_the_proposal_is_never_written_over_the_deployments_file(self):
        self.ready()
        before = open(self.partner).read()
        self.out = os.path.dirname(self.partner)
        with self.assertRaises(ses.Refused):
            self.propose()
        self.assertEqual(open(self.partner).read(), before)

    def test_an_old_block_at_the_end_without_a_newline_is_replaced_whole(self):
        self.ready()
        with open(self.partner, "a") as f:
            f.write("acceptance:\n  status: stale\n  session: old")
        proposed, _ = self.propose()
        import yaml
        d = yaml.safe_load(open(proposed))
        self.assertEqual(d["acceptance"]["session"], self.m["session_id"])
        self.assertNotIn("session: old", open(proposed).read())

    def test_nothing_is_proposed_without_the_whole_scope_cleanup_and_both_approvals(self):
        for why, kw, approvals in (("our cleanup", dict(cleanup=False), None),
                                   ("their cleanup", dict(peer_cleanup=False), None),
                                   ("a level that moves nothing", dict(level="connected"), None),
                                   ("one approver", {}, ("xaa=Operator A",))):
            with self.subTest(why=why):
                self.setUp()
                self.ready(**kw)
                with self.assertRaises(ses.Refused):
                    self.propose(approvals) if approvals else self.propose()
                self.assertFalse(os.path.exists(os.path.join(self.out, "xab.yaml")))

    def test_approver_names_are_written_exactly_and_control_characters_refused(self):
        self.ready()
        proposed, _ = self.propose(("xaa=Operator \"A\" \\ the first", "xab=Operator B"))
        import yaml
        self.assertEqual(yaml.safe_load(open(proposed))["acceptance"]["approved_by"]["xaa"],
                         'Operator "A" \\ the first')
        for bad in ("xaa=Operator\nlevel: exchanging", "xaa=Op\x1berator"):
            with self.subTest(bad=bad), self.assertRaises(ses.Refused):
                self.propose((bad, "xab=Operator B"))

    def test_an_open_bilateral_scope_is_refused(self):
        self.ready()
        with open(os.path.join(self.evidence, "result.json")) as f:
            r = json.load(f)
        r["checks"].append({"name": "refusal/acl-trace", "required": True, "verdict": "BLOCKED", "observations": {}})
        with open(os.path.join(self.evidence, "result.json"), "w") as f:
            json.dump(r, f)
        with self.assertRaises(ses.Refused) as e:
            self.propose()
        self.assertIn("refusal/acl-trace", str(e.exception))

    def test_freshness_goes_stale_or_degraded_and_writes_nothing(self):
        self.ready()
        proposed, _ = self.propose()
        now = utc_now_inside(self.m)
        self.assertEqual(ses.freshness(proposed, self.ag, now=now)[0], "current")
        self.assertEqual(ses.freshness(proposed, self.ag, now=now + datetime.timedelta(days=60))[0], "stale")
        with open(os.path.join(self.evidence, "result.json")) as f:
            r = json.load(f)
        r.update({"verdict": "FAIL", "why": "1 check(s) failed: delivery/inbound/release/all"})
        r["run"]["ended_at"] = ses.ts(now + datetime.timedelta(hours=1))
        with open(os.path.join(self.evidence, "result.json"), "w") as f:
            json.dump(r, f)
        self.assertEqual(ses.freshness(proposed, self.ag, self.evidence, now=now)[0], "degraded")
        ag = json.load(open(self.ag))
        ag["expires"] = "2027-01-01"
        json.dump(ag, open(self.ag, "w"))
        before = open(proposed).read()
        self.assertEqual(ses.freshness(proposed, self.ag, now=now)[0], "stale")
        self.assertEqual(open(proposed).read(), before)
        self.assertEqual(ses.freshness(self.partner, self.ag, now=now)[0], "none")


def utc_now_inside(m):
    """`combine` reads real time nowhere; imports in these tests need a time inside the window."""
    return ses.parse_ts(m["not_before"]) + datetime.timedelta(minutes=10)


if __name__ == "__main__":
    unittest.main()
