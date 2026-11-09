"""acceptance.sh against a fabric in one process: it passes only when every check ran
and passed, and a skipped check is never a pass.

Run: python3 -m pytest ops/federation/standup/test_acceptance.py
(or python3 -m unittest from this directory). Needs bash, curl and openssl.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import importlib.util

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "acceptance.sh"

_spec = importlib.util.spec_from_file_location("acceptance_fakefabric", HERE / "acceptance_fakefabric.py")
fakefabric = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fakefabric)

# Keys only this run sets; nothing the developer's shell exports may leak in.
SCRUB = [k for k in os.environ if k.startswith("FED_")]


@unittest.skipUnless(shutil.which("curl") and shutil.which("openssl"), "needs curl and openssl")
class AcceptanceRun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="acceptance-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.ag = fakefabric.agreement()
        self.fabric = fakefabric.Fabric(self.ag, "xaa", self.tmp)
        self.addCleanup(self.fabric.stop)

    def run_acceptance(self, drop=(), extra=None, agreement=None):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(agreement or self.ag, f)
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        env.update(self.fabric.start())
        for k in drop:
            env.pop(k, None)
        env.update({"FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3"})
        env.update(extra or {})
        p = subprocess.run(["bash", str(SCRIPT), path, "--as", "xaa"], env=env,
                           capture_output=True, text=True, timeout=240)
        return p.returncode, p.stdout + p.stderr

    def bundle(self):
        with open(os.path.join(self.tmp, "evidence", "acceptance", "result.json")) as f:
            return json.load(f)

    def verdicts(self):
        return {c["name"]: c["verdict"] for c in self.bundle()["checks"]}

    def counts(self, out):
        line = [x for x in out.splitlines() if x.startswith("acceptance-counts:")][-1]
        return dict(kv.split("=", 1) for kv in line.split()[1:])

    def test_everything_works_and_passes(self):
        rc, out = self.run_acceptance()
        c = self.counts(out)
        self.assertEqual((rc, c["fail"], c["skip"], c["blocked"]), (0, "0", "0", "0"), out)
        self.assertNotIn("ACCEPTANCE FAILED", out)
        b = self.bundle()
        self.assertEqual((b["schema"], b["verdict"], b["exit"]), ("goat-federation-acceptance/v1", "PASS", 0))
        self.assertEqual(set(self.verdicts().values()), {"PASS"})
        names = set(self.verdicts())
        for want in ("delivery/inbound/release/all", "delivery/outbound/release/xab", "republisher-boundary/outbound",
                     "refusal/control-allowed", "refusal/out-of-lane-denied", "refusal/forged-name",
                     "refusal/impersonation", "roundtrip/outbound/release/all", "spire/svid-chains"):
            self.assertIn(want, names)
        inbound = [c for c in b["checks"] if c["name"] == "delivery/inbound/release/all"][0]
        self.assertEqual(inbound["direction"], "inbound")
        self.assertEqual(inbound["observations"]["received"], inbound["observations"]["sent"])
        denied = [c for c in b["checks"] if c["name"] == "refusal/out-of-lane-denied"][0]
        self.assertIn("is unauthorized to Put (ingress) on ops/fabric/", denied["observations"]["trace"])

    def test_no_counterparty_surface_is_not_a_pass(self):
        # The production shape before this change: only our side, one skip, exit 0.
        rc, out = self.run_acceptance(drop=["FED_THEIRS_REST"])
        c = self.counts(out)
        self.assertEqual(c["fail"], "0", out)
        self.assertNotEqual(c["blocked"], "0", out)
        self.assertEqual(rc, 3, out)
        self.assertIn("is not a pass", out)
        v = self.verdicts()
        self.assertEqual(v["delivery/inbound/release/all"], "BLOCKED")
        self.assertEqual(v["delivery/outbound/release/xab"], "BLOCKED")
        self.assertEqual(v["roundtrip/inbound"], "BLOCKED")
        # What our side can prove alone, it proves.
        self.assertEqual(v["roundtrip/outbound/release/all"], "PASS")
        self.assertEqual(self.bundle()["verdict"], "INCOMPLETE")
        self.assertFalse(self.bundle()["inputs"]["theirs_rest"])

    def test_strict_off_does_not_turn_a_skip_into_a_pass(self):
        rc, out = self.run_acceptance(drop=["FED_SPIRE_OURS"], extra={"FED_ACCEPTANCE_STRICT": "0"})
        self.assertEqual(rc, 3, out)

    def test_a_missing_trace_is_a_skip_not_a_pass(self):
        rc, out = self.run_acceptance(drop=["FED_EDGE_LOG_OURS"])
        self.assertEqual(rc, 3, out)
        self.assertIn("the ACL trace was not read", out)
        self.assertEqual(self.verdicts()["refusal/acl-trace"], "BLOCKED")

    def test_a_check_whose_evidence_cannot_be_recorded_fails_the_run(self):
        # A copy of the runner whose record step refuses one check: the bundle would
        # lack it, so the run must not exit with the bundle's own verdict.
        d = os.path.join(self.tmp, "runner")
        os.makedirs(d)
        shutil.copy(SCRIPT, os.path.join(d, "acceptance.sh"))
        src = open(HERE / "acceptance_evidence.py").read()
        src = src.replace("    try:\n        if a.cmd == \"record\":",
                          "    if a.cmd == \"record\" and a.name.startswith(\"spire\"):\n        return 2\n"
                          "    try:\n        if a.cmd == \"record\":", 1)
        self.assertIn('a.name.startswith("spire")', src)
        open(os.path.join(d, "acceptance_evidence.py"), "w").write(src)
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        env.update(self.fabric.start())
        env.update({"FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3"})
        p = subprocess.run(["bash", os.path.join(d, "acceptance.sh"), path, "--as", "xaa"], env=env,
                           capture_output=True, text=True, timeout=240)
        out = p.stdout + p.stderr
        self.assertEqual(p.returncode, 2, out)
        self.assertIn("could not be recorded", out)

    def test_an_unwritable_evidence_directory_stops_the_run(self):
        blocker = os.path.join(self.tmp, "not-a-directory")
        open(blocker, "w").close()
        rc, out = self.run_acceptance(extra={"FED_EVIDENCE_DIR": os.path.join(blocker, "x")})
        self.assertEqual(rc, 2, out)
        self.assertIn("cannot write the evidence directory", out)

    def test_a_failure_still_exits_1(self):
        self.fabric.faults.republisher_down.add("xab")
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        self.assertIn("not delivered inbound", out)
        self.assertEqual(self.verdicts()["delivery/inbound/release/all"], "FAIL")
        self.assertEqual(self.verdicts()["refusals"], "SKIP")
        self.assertEqual(self.bundle()["exit"], 1)

    # --- what the agreement-derived checks add ------------------------------
    def test_every_direction_and_set_is_checked_including_the_inbound_round_trip(self):
        rc, out = self.run_acceptance()
        v = self.verdicts()
        for name in ("delivery/inbound/release/all", "delivery/inbound/release/xaa", "delivery/outbound/release/all",
                     "delivery/outbound/release/xab", "republisher-boundary/outbound", "republisher-boundary/inbound",
                     "roundtrip/outbound/release/all", "roundtrip/outbound/release/xab",
                     "roundtrip/inbound/release/all", "roundtrip/inbound/release/xaa"):
            self.assertEqual(v.get(name), "PASS", (name, out))
        d = [c for c in self.bundle()["checks"] if c["name"] == "delivery/outbound/release/xab"][0]
        self.assertEqual(d["observations"]["sent_sha256"], d["observations"]["received_sha256"])
        self.assertIn(self.bundle()["run"]["id"], d["observations"]["sent"])

    def test_a_dead_rest_surface_is_blocked_never_a_silent_pass(self):
        # Before: a read that failed looked like "nothing arrived", which is what every
        # refusal check wants to see.
        self.fabric.faults.broken_get.add("xaa")
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertNotEqual(rc, 0, out)
        self.assertNotIn("PASS", [v[n] for n in v if n.startswith(("out-of-lane/", "republisher-boundary/", "refusal/"))], out)
        self.assertEqual(v["delivery/inbound/release/all"], "BLOCKED", out)
        self.assertIn("HTTP 500", out)

    def test_a_surface_that_answers_every_key_is_not_fresh_evidence(self):
        self.fabric.faults.answers_everything["xaa"] = b"retained-from-an-earlier-run"
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["fresh-keys"], "FAIL", out)

    def test_other_bytes_on_arrival_fail_with_both_digests(self):
        self.fabric.faults.mangle_inbound = True
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        c = [c for c in self.bundle()["checks"] if c["name"] == "delivery/inbound/release/all"][0]
        self.assertEqual(c["verdict"], "FAIL")
        self.assertIn("other bytes", c["reason"])
        self.assertNotEqual(c["observations"]["sent_sha256"], c["observations"]["received_sha256"])

    def test_their_republisher_lifting_an_unoffered_set_fails(self):
        self.fabric.faults.lift_everything.add("xab")
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["republisher-boundary/inbound"], "FAIL", out)
        self.assertEqual(self.verdicts()["republisher-boundary/outbound"], "PASS", out)

    def test_a_silent_trace_is_a_failure_not_a_pass(self):
        self.fabric.faults.trace_silent = True
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        v = self.verdicts()
        self.assertEqual(v["refusal/control-allowed"], "FAIL", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "SKIP", out)

    # --- refusal tied to the control's session --------------------------------
    def test_the_denials_are_recorded_for_the_controls_session(self):
        rc, out = self.run_acceptance(extra={"FED_PROBE_THEIRS_ZID": self.fabric.agreed_zid})
        self.assertEqual(rc, 0, out)
        b = {c["name"]: c for c in self.bundle()["checks"]}
        for name in ("refusal/control-allowed", "refusal/out-of-lane-denied", "refusal/spoof-denied"):
            self.assertEqual(b[name]["verdict"], "PASS", (name, out))
            self.assertEqual(b[name]["observations"]["session"], self.fabric.agreed_zid)

    def test_a_denial_recorded_for_another_session_fails(self):
        self.fabric.faults.deny_from_other_session = True
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertEqual(rc, 1, out)
        self.assertEqual(v["refusal/control-allowed"], "PASS", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "FAIL", out)
        self.assertIn("another session", out)

    def test_an_allow_from_a_session_that_is_not_the_probe_fails_the_control(self):
        rc, out = self.run_acceptance(extra={"FED_PROBE_THEIRS_ZID": "b" * 32})
        v = self.verdicts()
        self.assertEqual(rc, 1, out)
        self.assertEqual(v["refusal/control-allowed"], "FAIL", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "SKIP", out)

    def test_decisions_without_a_session_id_cannot_prove_a_denial(self):
        self.fabric.faults.trace_without_session = True
        # Even with the probe's id given: an allow line naming no session is evidence
        # that cannot be read, not an allow from another session.
        rc, out = self.run_acceptance(extra={"FED_PROBE_THEIRS_ZID": self.fabric.agreed_zid})
        v = self.verdicts()
        self.assertEqual(rc, 3, out)
        self.assertEqual(v["refusal/control-allowed"], "PASS", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "BLOCKED", out)

    def test_a_probe_session_the_edge_never_named_is_blocked(self):
        # Our edge's log holds no line at all for the forged and impersonating probes'
        # sessions: their refusal cannot be shown, so it is not a pass.
        self.fabric.faults.silent_probes.update({"forged", "impersonate"})
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertEqual(v["refusal/forged-name"], "BLOCKED", out)
        self.assertEqual(v["refusal/impersonation"], "BLOCKED", out)
        self.assertEqual(rc, 3, out)

    def test_an_unmatched_session_named_once_before_the_run_still_counts(self):
        # The edge names a peer matching no subject once, at connect. A probe that
        # connected before the run is named only by that earlier line.
        with open(self.fabric.trace_path, "w") as f:
            for z in self.fabric.zids.values():
                f.write("2026-11-08T00:00:00Z INFO access_control: %s did not match any configured ACL subject. "
                        "Default permission `Deny` will be applied on all messages\n" % z)
        self.fabric.faults.silent_probes.update({"forged", "impersonate"})
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertEqual(v["refusal/forged-name"], "PASS", out)
        self.assertEqual(v["refusal/impersonation"], "PASS", out)
        self.assertEqual(rc, 0, out)

    def test_a_send_only_agreement_is_not_applicable_inbound_and_passes(self):
        ag = fakefabric.agreement(theirs=())
        self.fabric = fakefabric.Fabric(ag, "xaa", self.tmp)
        self.addCleanup(self.fabric.stop)
        rc, out = self.run_acceptance(agreement=ag)
        v = self.verdicts()
        self.assertEqual(rc, 0, out)
        for name in ("delivery/inbound", "persistence/inbound", "republisher-boundary/inbound", "roundtrip/inbound",
                     "roundtrip/outbound", "refusal/control-allowed"):
            self.assertEqual(v[name], "NOT APPLICABLE", (name, out))
        self.assertEqual(v["delivery/outbound/release/all"], "PASS", out)
        self.assertEqual(v["refusal/forged-name"], "PASS", out)
        na = [c for c in self.bundle()["checks"] if c["verdict"] == "NOT APPLICABLE"]
        self.assertTrue(all(not c["required"] and c["reason"] for c in na))

    def test_a_receive_only_agreement_checks_inbound_only(self):
        ag = fakefabric.agreement(ours=())
        self.fabric = fakefabric.Fabric(ag, "xaa", self.tmp)
        self.addCleanup(self.fabric.stop)
        rc, out = self.run_acceptance(agreement=ag)
        v = self.verdicts()
        self.assertEqual(rc, 0, out)
        self.assertEqual(v["delivery/outbound"], "NOT APPLICABLE", out)
        self.assertEqual(v["delivery/inbound/release/xaa"], "PASS", out)
        self.assertEqual(v["refusal/control-allowed"], "PASS", out)

    def test_an_agreement_with_no_flow_fails(self):
        ag = fakefabric.agreement(ours=(), theirs=())
        rc, out = self.run_acceptance(agreement=ag)
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["lanes"], "FAIL", out)

    # --- persistent reception ----------------------------------------------
    def test_every_offered_set_persists_on_the_store_alone(self):
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertEqual(rc, 0, out)
        for name in ("persistence/inbound/release/all", "persistence/inbound/release/xaa",
                     "persistence/outbound/release/all", "persistence/outbound/release/xab"):
            self.assertEqual(v.get(name), "PASS", (name, out))
        c = [c for c in self.bundle()["checks"] if c["name"] == "persistence/inbound/release/all"][0]
        scope = json.loads(c["observations"]["scope"])
        self.assertEqual(scope["outcome"], "ready")
        self.assertEqual(scope["details"]["store_subscriptions"], ["from/xab/**"])
        self.assertTrue(c["observations"]["key"].endswith("/persist"))

    def test_a_diagnostic_subscriber_masking_a_refused_store_is_not_a_pass(self):
        # The defect: the edge refuses the store's own subscription, and a session
        # someone left subscribed carries the samples. Delivery reads look fine.
        self.fabric.faults.storage_interest_refused.add("xaa")
        self.fabric.faults.diagnostic["xaa"] = "from/xab/**"
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertEqual(v["delivery/inbound/release/all"], "PASS", out)
        self.assertEqual(v["persistence/inbound/release/all"], "BLOCKED", out)
        self.assertEqual(v["persistence/outbound/release/all"], "PASS", out)
        self.assertEqual(rc, 3, out)
        self.assertIn("FED_TEST_SUBSCRIBERS_PAUSE_OURS", out)
        self.assertEqual(v["refusal/out-of-lane-denied"], "PASS", out)

    def test_with_the_test_subscriber_stopped_the_refused_store_fails(self):
        self.fabric.faults.storage_interest_refused.add("xaa")
        self.fabric.faults.diagnostic["xaa"] = "from/xab/**"
        flag = os.path.join(self.tmp, "paused-xaa")
        rc, out = self.run_acceptance(extra={"FED_TEST_SUBSCRIBERS_PAUSE_OURS": "touch '%s'" % flag,
                                             "FED_TEST_SUBSCRIBERS_RESUME_OURS": "rm -f '%s'" % flag})
        v = self.verdicts()
        self.assertEqual(rc, 1, out)
        self.assertEqual(v["persistence/inbound/release/all"], "FAIL", out)
        self.assertFalse(os.path.exists(flag), "the test subscriber was not started again")

    def test_a_pause_command_without_a_resume_command_is_refused(self):
        flag = os.path.join(self.tmp, "paused-xaa")
        rc, out = self.run_acceptance(extra={"FED_TEST_SUBSCRIBERS_PAUSE_OURS": "touch '%s'" % flag})
        self.assertEqual(rc, 2, out)
        self.assertIn("FED_TEST_SUBSCRIBERS_RESUME_OURS", out)
        self.assertFalse(os.path.exists(flag))

    def test_a_subscriber_on_other_keys_does_not_block(self):
        # The round-trip responder subscribes to fedcheck keys only; it carries no
        # persistence sample, so it is no reason to stop.
        self.fabric.faults.diagnostic["xaa"] = "from/xab/**/fedcheck/*"
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.verdicts()["persistence/inbound/release/all"], "PASS", out)

    def test_a_closed_admin_space_is_blocked_not_a_missing_store(self):
        self.fabric.faults.admin_closed.add("xaa")
        rc, out = self.run_acceptance()
        v = self.verdicts()
        self.assertEqual(rc, 3, out)
        self.assertEqual(v["persistence/inbound/release/all"], "BLOCKED", out)
        self.assertIn("adminspace read", out)
        # A persistence row that could not run does not take the refusal rows with it.
        for name in ("republisher-boundary/inbound", "out-of-lane/inbound", "refusal/control-allowed",
                     "refusal/out-of-lane-denied", "refusal/forged-name"):
            self.assertEqual(v.get(name), "PASS", (name, out))
        self.assertNotIn("refusals", v)

    def test_no_store_for_the_lane_fails(self):
        self.fabric.faults.no_storage.add("xab")
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["persistence/outbound/release/all"], "FAIL", out)

    def test_persistence_without_a_counterparty_surface_is_blocked(self):
        rc, out = self.run_acceptance(drop=["FED_THEIRS_REST"])
        v = self.verdicts()
        self.assertEqual(rc, 3, out)
        self.assertEqual(v["persistence/inbound/release/all"], "BLOCKED", out)
        self.assertEqual(v["persistence/outbound/release/xab"], "BLOCKED", out)

    # --- existing partnerships before and after -----------------------------
    def existing(self):
        """xaa already peers with xac (sets release/all and release/xac each way)."""
        ag = fakefabric.agreement(them="xac", ours=("release/all/**", "release/xac/**"),
                                  theirs=("release/all/**", "release/xaa/**"))
        self.fabric.add_existing(ag)
        work = os.path.join(self.tmp, "peerings", "xac")
        os.makedirs(work)
        with open(os.path.join(work, "agreement.json"), "w") as f:
            json.dump(ag, f)
        return work

    def run_before(self, extra):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        env.update(self.fabric.start())
        env.update({"FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3"})
        env.update(extra)
        p = subprocess.run(["bash", str(SCRIPT), path, "--as", "xaa", "--before"], env=env,
                           capture_output=True, text=True, timeout=240)
        return p.returncode, p.stdout + p.stderr

    def test_no_existing_partnership_is_checked_unless_named(self):
        self.existing()
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 0, out)
        self.assertFalse([n for n in self.verdicts() if n.startswith("existing/")], out)

    def test_existing_partnerships_are_unchanged_before_and_after(self):
        extra = {"FED_EXISTING_WORKS": self.existing()}
        rc, out = self.run_before(extra)
        self.assertEqual(rc, 0, out)
        before = json.load(open(os.path.join(self.tmp, "evidence", "acceptance", "before", "result.json")))
        self.assertEqual({c["name"]: c["verdict"] for c in before["checks"]},
                         {"existing/xac/agreement": "PASS", "existing/xac/roundtrip/release/all": "PASS",
                          "existing/xac/roundtrip/release/xac": "PASS"})
        rc, out = self.run_acceptance(extra=extra)
        v = self.verdicts()
        self.assertEqual(rc, 0, out)
        for name in ("existing/xac/agreement", "existing/xac/roundtrip/release/all", "existing/xac/leak/release/xab"):
            self.assertEqual(v.get(name), "PASS", (name, out))

    def test_without_a_snapshot_from_before_the_agreement_row_is_blocked(self):
        rc, out = self.run_acceptance(extra={"FED_EXISTING_WORKS": self.existing()})
        self.assertEqual(rc, 3, out)
        self.assertEqual(self.verdicts()["existing/xac/agreement"], "BLOCKED", out)
        self.assertIn("--before", out)

    def test_a_changed_existing_agreement_fails(self):
        work = self.existing()
        self.assertEqual(self.run_before({"FED_EXISTING_WORKS": work})[0], 0)
        ag = json.load(open(os.path.join(work, "agreement.json")))
        ag["expires"] = "2027-01-01"
        json.dump(ag, open(os.path.join(work, "agreement.json"), "w"))
        rc, out = self.run_acceptance(extra={"FED_EXISTING_WORKS": work})
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["existing/xac/agreement"], "FAIL", out)

    def test_an_existing_partnership_broken_by_the_deployment_fails_and_says_so(self):
        work = self.existing()
        self.assertEqual(self.run_before({"FED_EXISTING_WORKS": work})[0], 0)
        self.fabric.faults.existing_down.add("xac")
        rc, out = self.run_acceptance(extra={"FED_EXISTING_WORKS": work})
        self.assertEqual(rc, 1, out)
        c = [c for c in self.bundle()["checks"] if c["name"] == "existing/xac/roundtrip/release/all"][0]
        self.assertEqual(c["verdict"], "FAIL")
        self.assertIn("before this deployment, and does not now", c["reason"])

    def test_a_set_offered_to_the_new_partner_only_must_not_reach_an_existing_one(self):
        work = self.existing()
        self.assertEqual(self.run_before({"FED_EXISTING_WORKS": work})[0], 0)
        self.fabric.faults.leak_to_existing.add("xac")
        rc, out = self.run_acceptance(extra={"FED_EXISTING_WORKS": work})
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["existing/xac/leak/release/xab"], "FAIL", out)

    def test_silence_on_an_existing_partner_that_receives_nothing_is_not_a_pass(self):
        work = self.existing()
        self.assertEqual(self.run_before({"FED_EXISTING_WORKS": work})[0], 0)
        self.fabric.faults.existing_down.add("xac")
        rc, out = self.run_acceptance(extra={"FED_EXISTING_WORKS": work})
        self.assertEqual(self.verdicts()["existing/xac/leak/release/xab"], "BLOCKED", out)

    def test_leakage_without_a_surface_is_an_optional_skip(self):
        work = self.existing()
        self.assertEqual(self.run_before({"FED_EXISTING_WORKS": work})[0], 0)
        rc, out = self.run_acceptance(drop=["FED_EXISTING_REST"], extra={"FED_EXISTING_WORKS": work})
        self.assertEqual(rc, 0, out)
        c = [c for c in self.bundle()["checks"] if c["name"] == "existing/xac/leak/release/xab"][0]
        self.assertEqual((c["verdict"], c["required"]), ("SKIP", False))

    def test_before_needs_a_named_partnership(self):
        rc, out = self.run_before({})
        self.assertEqual(rc, 2, out)

    def test_a_broken_round_trip_fails(self):
        self.fabric.faults.responder_down.add("xab")
        rc, out = self.run_acceptance(extra={})
        self.assertEqual(rc, 1, out)
        self.assertIn("did not complete", out)


if __name__ == "__main__":
    unittest.main()
