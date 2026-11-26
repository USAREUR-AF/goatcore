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

    # --- a sample the receiving edge drops ----------------------------------
    # The receiving edge passes at most one message per 1/hz on a lane and drops the
    # rest (ADR 1244), so a check's single sample can land just behind a flow sample
    # and be lost. The check sends a fresh value until one is stored.
    def check(self, name):
        return [c for c in self.bundle()["checks"] if c["name"] == name][0]

    def test_a_dropped_delivery_sample_is_resent_fresh_until_one_is_stored(self):
        self.fabric.faults.drop_samples["xaa"] = {r"from/xab/all/acceptance/[^/]+": 1}
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 0, out)
        c = self.check("delivery/inbound/release/all")
        self.assertEqual(c["verdict"], "PASS", out)
        o = c["observations"]
        sent = o["values_sent"].split()
        self.assertEqual((o["resent"], len(sent), len(set(sent))), ("1", 2, 2), o)
        self.assertEqual(o["received"], sent[-1])
        self.assertIn("1/hz", c["reason"])
        self.assertIn("10 ms at 100 Hz", c["reason"])

    def test_a_dropped_persistence_sample_is_resent_fresh_until_one_is_stored(self):
        self.fabric.faults.drop_samples["xab"] = {r"from/xaa/xab/acceptance/[^/]+/persist": 1}
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 0, out)
        c = self.check("persistence/outbound/release/xab")
        self.assertEqual(c["verdict"], "PASS", out)
        o = c["observations"]
        sent = o["values_sent"].split()
        self.assertEqual((o["resent"], len(sent), len(set(sent))), ("1", 2, 2), o)
        self.assertEqual(o["received"], sent[-1])
        self.assertIn("10 ms at 100 Hz", c["reason"])

    def test_a_check_stored_first_time_sends_once_and_names_no_rule(self):
        self.run_acceptance()
        for name in ("delivery/inbound/release/all", "persistence/outbound/release/xab"):
            c = self.check(name)
            self.assertEqual((c["verdict"], c["observations"]["resent"]), ("PASS", "0"), c)
            self.assertNotIn("1/hz", c["reason"])

    def test_a_lane_that_stores_nothing_fails_and_names_the_rule(self):
        self.fabric.faults.republisher_down.add("xab")
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 1, out)
        c = self.check("delivery/inbound/release/all")
        self.assertEqual(c["verdict"], "FAIL", out)
        sent = c["observations"]["values_sent"].split()
        self.assertEqual(len(set(sent)), 8, c)
        self.assertIn("10 ms at 100 Hz", c["reason"])
        self.assertIn("ADR 1244", c["reason"])

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

    def test_an_existing_agreement_with_malformed_names_is_not_used(self):
        work = self.existing()
        ag = json.load(open(os.path.join(work, "agreement.json")))
        marker = os.path.join(self.tmp, "ran")
        ag["flows"][0]["release"].append("release/x'$(touch %s)'/**" % marker)
        json.dump(ag, open(os.path.join(work, "agreement.json"), "w"))
        rc, out = self.run_before({"FED_EXISTING_WORKS": work})
        self.assertFalse(os.path.exists(marker), out)
        before = json.load(open(os.path.join(self.tmp, "evidence", "acceptance", "before", "result.json")))
        row = [c for c in before["checks"] if c["name"].startswith("existing/")][0]
        self.assertEqual(row["verdict"], "BLOCKED", out)
        self.assertIn("well-formed", row["reason"])

    def test_before_needs_a_named_partnership(self):
        rc, out = self.run_before({})
        self.assertEqual(rc, 2, out)

    # --- a bilateral session, each side on its own core only -----------------
    def run_side(self, side, manifest, evidence, full=False):
        """One side's runner, on its own systems only: its core and no counterparty
        surface; with `full`, also its own edge's trace, probes and SPIRE commands."""
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        fab = self.fabric.start()
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        if full:
            env.update(self.fabric.env_for(side))
        env.update({"FED_OURS_REST": fab["FED_OURS_REST"] if side == "xaa" else fab["FED_THEIRS_REST"],
                    "FED_ACCEPTANCE_SESSION": manifest, "FED_EVIDENCE_DIR": evidence, "FED_SESSION_WAIT": "2",
                    "FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3"})
        p = subprocess.run(["bash", str(SCRIPT), path, "--as", side], env=env, capture_output=True, text=True,
                           timeout=240)
        with open(os.path.join(evidence, "result.json")) as f:
            return p.returncode, p.stdout + p.stderr, json.load(f)

    def test_a_bilateral_session_exchanged_by_file(self):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        manifest = os.path.join(self.tmp, "session.json")
        tool = str(HERE / "acceptance_session.py")
        subprocess.run(["python3", tool, "new", "--agreement", path, "--us", "xaa", "--out", manifest], check=True,
                       capture_output=True)
        ev_a, ev_b = os.path.join(self.tmp, "ev-xaa"), os.path.join(self.tmp, "ev-xab")
        # xab first: nothing of ours yet, so its inbound rows wait and stay BLOCKED.
        rc, out, b = self.run_side("xab", manifest, ev_b)
        self.assertEqual(rc, 3, out)
        rows = {c["name"]: c for c in b["checks"]}
        self.assertEqual(rows["delivery/inbound/release/all"]["verdict"], "BLOCKED", out)
        self.assertEqual(rows["publish/outbound/release/all"]["verdict"], "PASS", out)
        # xaa: reads xab's samples, with xab's challenge, from its own core.
        rc, out, a = self.run_side("xaa", manifest, ev_a)
        rows = {c["name"]: c for c in a["checks"]}
        self.assertEqual(rows["delivery/inbound/release/all"]["verdict"], "PASS", out)
        self.assertEqual(rows["republisher-boundary/inbound"]["verdict"], "PASS", out)
        self.assertEqual(rows["delivery/outbound/release/all"]["verdict"], "BLOCKED", out)
        self.assertEqual(rows["delivery/outbound/release/all"]["observations"]["observed_by_peer"],
                         "delivery/inbound/release/all")
        self.assertEqual(a["session"]["id"], json.load(open(manifest))["session_id"])
        self.assertIn("acceptance.sh", a["software"])
        self.assertNotIn("theirs_rest", [k for k, v in a["inputs"].items() if v])
        # xab again: now it sees ours.
        rc, out, b = self.run_side("xab", manifest, ev_b)
        self.assertEqual({c["name"]: c["verdict"] for c in b["checks"]}["delivery/inbound/release/all"], "PASS", out)
        # Exchanged by file: xaa imports xab's bundle; a second import is a replay.
        imp = subprocess.run(["python3", tool, "import", "--manifest", manifest, "--us", "xaa", "--evidence", ev_a,
                              os.path.join(ev_b, "result.json")], capture_output=True, text=True)
        self.assertEqual(imp.returncode, 0, imp.stderr)
        again = subprocess.run(["python3", tool, "import", "--manifest", manifest, "--us", "xaa", "--evidence", ev_a,
                                os.path.join(ev_b, "result.json")], capture_output=True, text=True)
        self.assertEqual(again.returncode, 2, again.stdout)
        self.assertIn("already imported", again.stderr)
        co = subprocess.run(["python3", tool, "combine", "--manifest", manifest, "--us", "xaa", "--evidence", ev_a],
                            capture_output=True, text=True)
        combined = json.load(open(os.path.join(ev_a, "bilateral", json.load(open(manifest))["session_id"] + ".json")))
        row = [r for r in combined["rows"] if r["name"] == "delivery/outbound/release/all"][0]
        self.assertEqual(row["status"], "PASS (peer-reported)", co.stdout)
        # Each side ran with its core only: no edge trace, probes or SPIRE commands, so
        # its refusal and SPIRE rows are BLOCKED or SKIPPED and the bilateral scope is open.
        self.assertEqual((co.returncode, combined["verdict"]), (3, "INCOMPLETE"), co.stdout)

    def test_a_session_sample_is_staged_when_our_core_accepts_it_and_refused_when_it_does_not(self):
        # A core declares no store on the staging name release/** (the fake fabric
        # models that), so the runner cannot read its own sample back there: the sample
        # is staged when our core's REST surface accepts the PUT, and only the partner's
        # runner observes it arriving.
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        manifest = os.path.join(self.tmp, "session.json")
        subprocess.run(["python3", str(HERE / "acceptance_session.py"), "new", "--agreement", path, "--us", "xaa",
                        "--out", manifest], check=True, capture_output=True)
        rc, out, a = self.run_side("xaa", manifest, os.path.join(self.tmp, "ev-ok"))
        row = {c["name"]: c for c in a["checks"]}["publish/outbound/release/all"]
        self.assertEqual(row["verdict"], "PASS", out)
        self.assertEqual(row["observations"].get("http_status"), "200", row)
        self.fabric.faults.refuse_put.add("xaa")
        rc, out, a = self.run_side("xaa", manifest, os.path.join(self.tmp, "ev-refused"))
        row = {c["name"]: c for c in a["checks"]}["publish/outbound/release/all"]
        self.assertEqual(row["verdict"], "FAIL", out)
        self.assertIn("403", out)

    def test_two_operators_each_on_their_own_systems_reach_a_bilateral_pass(self):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        manifest = os.path.join(self.tmp, "session.json")
        tool = str(HERE / "acceptance_session.py")
        subprocess.run(["python3", tool, "new", "--agreement", path, "--us", "xaa", "--out", manifest], check=True,
                       capture_output=True)
        ev = {"xaa": os.path.join(self.tmp, "ev-xaa"), "xab": os.path.join(self.tmp, "ev-xab")}
        self.run_side("xab", manifest, ev["xab"], full=True)           # sees nothing of xaa's yet
        rc, out, a = self.run_side("xaa", manifest, ev["xaa"], full=True)
        rows = {c["name"]: c["verdict"] for c in a["checks"]}
        for name in ("delivery/inbound/release/all", "persistence/inbound/release/all", "out-of-lane/inbound",
                     "republisher-boundary/inbound", "refusal/control-allowed", "refusal/out-of-lane-denied",
                     "refusal/forged-name", "roundtrip/outbound/release/all", "spire/svid-chains"):
            self.assertEqual(rows.get(name), "PASS", (name, out))
        self.assertEqual(rows["persistence/outbound/release/all"], "BLOCKED", out)
        rc, out, b = self.run_side("xab", manifest, ev["xab"], full=True)  # now sees xaa's
        self.assertEqual({c["name"]: c["verdict"] for c in b["checks"]}["refusal/control-allowed"], "PASS", out)
        for us, them in (("xaa", "xab"), ("xab", "xaa")):
            imp = subprocess.run(["python3", tool, "import", "--manifest", manifest, "--us", us, "--evidence", ev[us],
                                  os.path.join(ev[them], "result.json")], capture_output=True, text=True)
            self.assertEqual(imp.returncode, 0, imp.stderr)
            co = subprocess.run(["python3", tool, "combine", "--manifest", manifest, "--us", us, "--evidence", ev[us]],
                                capture_output=True, text=True)
            combined = json.load(open(os.path.join(ev[us], "bilateral", json.load(open(manifest))["session_id"] + ".json")))
            self.assertEqual((co.returncode, combined["verdict"]), (0, "PASS"),
                             (us, combined["open"], combined["failed"]))

    # --- a derived session: no manifest file, each result signed (§6.2, §6.3) ----
    def derived_env(self, side, evidence, full=False, wait="20", extra=None, own=False):
        fab = self.fabric.start()
        env = {k: v for k, v in os.environ.items() if k not in SCRUB}
        if own:
            env.update(self.fabric.own_env(side))
        elif full:
            env.update(self.fabric.env_for(side))
        keys = Path(HERE).parents[1] / "federation/vectors/testkeys"
        double = "python3 %s" % (HERE / "acceptance_sig_double.py")
        env.update({"FED_OURS_REST": fab["FED_OURS_REST"] if side == "xaa" else fab["FED_THEIRS_REST"],
                    "FED_ACCEPTANCE_ATTEMPT": "1", "FED_EVIDENCE_DIR": evidence, "FED_SESSION_WAIT": wait,
                    "FED_ACCEPTANCE_WAIT": "0.3", "FED_ACCEPTANCE_GAP": "0", "FED_ACCEPTANCE_RT_TIMEOUT": "3",
                    "FED_ACCEPTANCE_SIGNER": double + " sign", "FED_ACCEPTANCE_VERIFIER": double + " verify",
                    "FED_TEST_SIG_SEED": str(keys / ("%s.seed.hex" % side)),
                    "FED_TEST_SIG_KEYS": ",".join("%s=%s" % (s, (keys / ("%s.public.b64" % s)).read_text().strip())
                                                  for s in ("xaa", "xab"))})
        env.update(extra or {})
        return env

    def derived_side(self, side, evidence, script=None, **kw):
        path = os.path.join(self.tmp, "agreement.json")
        if not os.path.exists(path):
            with open(path, "w") as f:
                json.dump(self.ag, f)
        return subprocess.Popen(["bash", str(script or SCRIPT), path, "--as", side], env=self.derived_env(side, evidence, **kw),
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)

    def both_derived(self, delay=1.5, after=None, **kw):
        """Both sides' phase 14, the second started `delay` seconds after the first,
        each on its own systems only: they overlap, as the rule in §6.2 asks. With
        `after`, the second starts once the first has printed that text instead: a
        case that needs the first side to have read something before the second
        overwrites it waits for that, not for a time a loaded machine can overrun."""
        import threading
        import time
        second = dict(kw, **kw.pop("second", {})) if "second" in kw else kw
        ev = {"xaa": os.path.join(self.tmp, "ev-xaa"), "xab": os.path.join(self.tmp, "ev-xab")}
        a = self.derived_side("xaa", ev["xaa"], full=True, **kw)
        lines_a, seen = [], threading.Event()

        def drain():
            for line in a.stdout:
                lines_a.append(line)
                if after and after in line:
                    seen.set()
            seen.set()  # xaa ended: start xab anyway, and let the assertions say what xaa printed
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        if after:
            self.assertTrue(seen.wait(timeout=240), "xaa printed neither %r nor its end in 240 s" % after)
        else:
            time.sleep(delay)
        b = self.derived_side("xab", ev["xab"], full=True, **second)
        out_b = b.communicate(timeout=240)[0]
        a.wait(timeout=240)
        reader.join(timeout=60)
        return ev, (a.returncode, "".join(lines_a)), (b.returncode, out_b)

    def bilateral(self, evidence):
        d = os.path.join(evidence, "bilateral")
        files = [f for f in os.listdir(d) if f.endswith(".json")] if os.path.isdir(d) else []
        self.assertEqual(len(files), 1, files)
        with open(os.path.join(d, files[0])) as f:
            return json.load(f)

    def test_derived_both_sides_publish_sign_and_verify_each_others_result(self):
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived()
        for side, rc, out, them in (("xaa", rc_a, out_a, "xab"), ("xab", rc_b, out_b, "xaa")):
            combined = self.bilateral(ev[side])
            self.assertEqual((rc, combined["verdict"]), (0, "PASS"), (side, out, combined["open"], combined["failed"]))
            self.assertTrue(combined["authenticated"], out)
            pub = (Path(HERE).parents[1] / "federation/vectors/testkeys" / ("%s.public.b64" % them)).read_text().strip()
            self.assertEqual(combined["peer_key"]["b64"], pub)
            import base64, hashlib
            self.assertEqual(combined["peer_key"]["sha256"], hashlib.sha256(base64.b64decode(pub)).hexdigest())
            self.assertNotIn("FED_ACCEPTANCE_SESSION", out)

    def test_derived_a_partner_that_never_runs_stays_blocked_exit_3(self):
        p = self.derived_side("xaa", os.path.join(self.tmp, "ev-xaa"), wait="3")
        out = p.communicate(timeout=120)[0]
        self.assertEqual(p.returncode, 3, out)
        self.assertIn("waiting for xab's phase 14", out)
        self.assertFalse(os.path.isdir(os.path.join(self.tmp, "ev-xaa", "bilateral")), out)

    def test_derived_each_run_draws_a_new_challenge_and_an_earlier_runs_partner_sample_teaches_nothing(self):
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived()
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        with open(os.path.join(ev["xaa"], "result.json")) as f:
            first = json.load(f)["session"]["challenge"]
        # xaa again, alone, in the same session: a new challenge. xab's samples and
        # result are still on xaa's core, from xab's run that started before xaa's
        # start minus twice the wait: an earlier run's, so its challenge is not
        # learned, no result is published or verified, and the run stays open. The
        # BLOCKED delivery rows say an earlier run's sample is there. An earlier run
        # is one that started more than twice the wait before this one.
        import time
        time.sleep(8)
        p = self.derived_side("xaa", ev["xaa"], full=True, wait="3")
        out = p.communicate(timeout=120)[0]
        with open(os.path.join(ev["xaa"], "result.json")) as f:
            second = json.load(f)["session"]
        self.assertRegex(second["challenge"], r"^[0-9a-f]{32}$")
        self.assertNotEqual(second["challenge"], first)
        with open(os.path.join(ev["xab"], "result.json")) as f:
            self.assertEqual(second["id"], json.load(f)["session"]["id"])
        self.assertEqual(p.returncode, 3, out)
        self.assertIn("from an earlier run", out)
        self.assertIn("BILATERAL OPEN", out)
        self.assertNotIn("FAIL", out.replace("ACCEPTANCE FAILED", ""))
        inbound = {n: r for n, r in self.checks(ev["xaa"]).items() if n.startswith("delivery/inbound/")}
        self.assertTrue(inbound, out)
        for name, row in inbound.items():
            self.assertEqual(row["verdict"], "BLOCKED", (name, out))
            self.assertIn("earlier run", row["reason"], (name, out))

    # A partner run's stamp, `<them>/run` beside its sample, carries the challenge and
    # the run's start time in one value (§6.3). These cases put an earlier run's
    # samples and stamp on xaa's core, then break one step of xab's current run.
    OLD_CHALLENGE = "0123456789abcdef0123456789abcdef"

    def session_id(self):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        der = os.path.join(self.tmp, "derived.json")
        subprocess.run(["python3", str(HERE / "acceptance_session.py"), "derive", "--agreement", path, "--us", "xaa",
                        "--attempt", "1", "--out", der], check=True, capture_output=True)
        return json.loads(Path(der).read_text())["session_id"]

    def earlier_run_on_xaa(self, stamp=True, started_ago=100000):
        """xab's samples from an earlier run on xaa's core: another challenge and,
        when `stamp`, that run's stamp with a start time `started_ago` seconds past."""
        import time
        sid, old = self.session_id(), self.OLD_CHALLENGE
        started = int(time.time()) - started_ago
        stale = {}
        for st in self.fabric.offered("xab"):
            name = st[len("release/"):]
            wire = "from/xab/%s/acceptance/%s/xab" % (name, sid)
            stale[wire] = ("xab-%s-%s" % (old, name)).encode()
            stale[wire + "/persist"] = ("xab-%s-%s-persist" % (old, name)).encode()
            if stamp:
                stale[wire + "/run"] = ("xab-%s-%s@%d" % (old, name, started)).encode()
        self.fabric.faults.stale["xaa"] = stale

    def stand_in(self, name, body):
        """A directory holding one executable stand-in, `name`, for PATH."""
        d = tempfile.mkdtemp(dir=self.tmp)
        path = os.path.join(d, name)
        with open(path, "w") as f:
            f.write("#!/usr/bin/env bash\n" + body)
        os.chmod(path, 0o700)  # the owner runs it; nobody else needs to read it
        return {"PATH": d + os.pathsep + os.environ["PATH"]}

    def curl_stand_in(self, condition, action):
        """curl that runs `action` (bash) for a call matching `condition` (bash test
        over $put, 1 for a PUT, and $url), and is the real curl otherwise."""
        real = shutil.which("curl")
        return self.stand_in("curl", (
            'put=0; url=""\n'
            'for a in "$@"; do [ "$a" = PUT ] && put=1; case "$a" in http*) url="$a";; esac; done\n'
            'if %s; then %s; fi\n'
            'exec %s "$@"\n') % (condition, action, real))

    BARE_SAMPLE_PUT = '[ "$put" = 1 ] && [[ "$url" =~ /acceptance/[^/]+/[a-z]+$ ]]'
    STAMP_PUT = '[ "$put" = 1 ] && [[ "$url" == */run ]]'

    def checks(self, evidence):
        with open(os.path.join(evidence, "result.json")) as f:
            return {c["name"]: c for c in json.load(f)["checks"]}

    def assert_no_earlier_challenge_used(self, out_a, out_b):
        for out in (out_a, out_b):
            self.assertNotIn("GB-273", out)
            self.assertNotIn(self.OLD_CHALLENGE, out)

    def test_derived_a_partner_sample_left_by_an_earlier_run_is_ignored_and_both_verdicts_agree(self):
        """A sample the partner's earlier run left on our core carries that run's
        challenge, and that run's stamp says when it started; a run ignores a stamp
        from before its own start minus twice FED_SESSION_WAIT and waits for this
        run's, so our signed result echoes the partner's current challenge and both
        sides reach the same verdict (§6.3)."""
        # xab starts once xaa has said it read the earlier run's sample, so xab's own
        # stamp cannot replace that sample before xaa reads it, however slow xaa is.
        self.earlier_run_on_xaa()
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived(after="from an earlier run")
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        for side in ("xaa", "xab"):
            self.assertEqual(self.bilateral(ev[side])["verdict"], "PASS", side)
        self.assert_no_earlier_challenge_used(out_a, out_b)
        self.assertIn("from an earlier run", out_a)

    def test_derived_each_run_stamps_its_challenge_and_start_time_in_one_value(self):
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived()
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        result = json.loads(Path(ev["xaa"], "result.json").read_text())["session"]
        st = self.fabric.offered("xaa")[0]
        name = st[len("release/"):]
        stamp = self.fabric.cores["xab"].store.get("from/xaa/%s/acceptance/%s/xaa/run" % (name, result["id"]))
        self.assertIsNotNone(stamp, "xaa's stamp crossed to xab's core with its samples")
        self.assertRegex(stamp.decode(), r"^xaa-%s-%s@[0-9]{10}$" % (result["challenge"], name))

    def test_derived_a_partner_sample_whose_write_failed_teaches_no_earlier_challenge(self):
        # xab's stamp lands but its sample's write fails, beside an earlier run's
        # sample: the challenge comes from the stamp, the earlier sample is named as
        # an earlier run's (BLOCKED), and nothing echoes or refuses another challenge.
        self.earlier_run_on_xaa()
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived(
            delay=3, wait="8", second={"extra": self.curl_stand_in(self.BARE_SAMPLE_PUT, "exit 28")})
        self.assert_no_earlier_challenge_used(out_a, out_b)
        rows = self.checks(ev["xaa"])
        inbound = [n for n in rows if n.startswith("delivery/inbound/")]
        self.assertTrue(inbound, out_a)
        for name in inbound:
            self.assertEqual(rows[name]["verdict"], "BLOCKED", (name, out_a))
            self.assertIn("earlier run", rows[name]["reason"], (name, out_a))
        # xab's own run fails its staged-sample check (its write failed); xaa's rows
        # carry no FAIL.
        self.assertNotIn("FAIL", [r["verdict"] for r in rows.values()], out_a)

    def test_derived_a_partner_sample_arriving_after_its_stamp_is_waited_for(self):
        # xab's sample lands 4 s after its stamp, beside an earlier run's sample: xaa
        # waits for the sample carrying the stamp's challenge. (xab's own read-back of
        # its staged sample runs before the late write, so xab's run is not judged.)
        self.earlier_run_on_xaa()
        delayed = "( sleep 4; exec %s \"$@\" ) >/dev/null 2>&1 & exit 0" % shutil.which("curl")
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived(
            delay=3, wait="12", second={"extra": self.curl_stand_in(self.BARE_SAMPLE_PUT, delayed)})
        self.assert_no_earlier_challenge_used(out_a, out_b)
        rows = self.checks(ev["xaa"])
        inbound = [n for n in rows if n.startswith("delivery/inbound/")]
        self.assertTrue(inbound, out_a)
        for name in inbound:
            self.assertEqual(rows[name]["verdict"], "PASS", (name, out_a))

    def test_derived_a_partner_stamp_arriving_after_its_sample_is_waited_for(self):
        self.earlier_run_on_xaa()
        delayed = "( sleep 4; exec %s \"$@\" ) >/dev/null 2>&1 & exit 0" % shutil.which("curl")
        _, (rc_a, out_a), (rc_b, out_b) = self.both_derived(
            delay=3, wait="12", second={"extra": self.curl_stand_in(self.STAMP_PUT, delayed)})
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        self.assert_no_earlier_challenge_used(out_a, out_b)

    def test_derived_a_failed_read_of_the_stamp_is_not_yet_never_an_older_runner(self):
        self.earlier_run_on_xaa()
        flag = os.path.join(self.tmp, "stamp-read-failed")
        once = '[ "$put" = 0 ] && [[ "$url" == */run ]] && [ ! -e %s ]' % flag
        _, (rc_a, out_a), (rc_b, out_b) = self.both_derived(
            delay=3, wait="12", extra=self.curl_stand_in(once, ": > %s; exit 7" % flag), second={"extra": None})
        self.assertTrue(os.path.exists(flag), "the stand-in failed a read of the stamp")
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        self.assert_no_earlier_challenge_used(out_a, out_b)

    def test_derived_a_partner_that_stages_no_stamp_is_read_as_before(self):
        # A partner on an older runner stages no stamp (here: its stamp's write is
        # dropped): its sample is read as before and both sides pass.
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived(
            second={"extra": self.curl_stand_in(self.STAMP_PUT, "exit 0")})
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        for side in ("xaa", "xab"):
            self.assertEqual(self.bilateral(ev[side])["verdict"], "PASS", side)

    # --- one run's challenge per run: xaa alone reads what xab's runs left -----
    NEW_CHALLENGE = "fedcba9876543210fedcba9876543210"

    def xab_keys(self, name, chal, started=None, sample=True, persist=True, ago=None):
        """What one of xab's runs leaves on xaa's core for set `name`: its stamp
        (when `started` or `ago` is given), its sample and its second sample.

        `ago` dates the stamp `ago` seconds before xaa first reads it, so it lies at
        that distance from xaa's own start however long xaa took to start. A start
        time taken before xaa ran (`started`) can fall outside twice the wait on a
        loaded machine, and the stamp is then read as an earlier run's."""
        import time
        wire = "from/xab/%s/acceptance/%s/xab" % (name, self.session_id())
        out = {}
        if ago is not None:
            out[wire + "/run"] = lambda: ("xab-%s-%s@%d" % (chal, name, int(time.time()) - ago)).encode()
        elif started is not None:
            out[wire + "/run"] = ("xab-%s-%s@%d" % (chal, name, started)).encode()
        if sample:
            out[wire] = ("xab-%s-%s" % (chal, name)).encode()
        if persist:
            out[wire + "/persist"] = ("xab-%s-%s-persist" % (chal, name)).encode()
        return out

    def xab_names(self):
        return [st[len("release/"):] for st in self.fabric.offered("xab")]

    def lone_xaa(self, before, then=None, wait="6"):
        """xaa's phase 14 alone, with `before` on its core from the start; `then` is
        written to its core once xaa first says it is waiting for xab, so xaa has
        read `before` at least once. Returns (rc, out, rows)."""
        self.fabric.faults.stale["xaa"] = before
        ev = os.path.join(self.tmp, "ev-xaa")
        p = self.derived_side("xaa", ev, wait=wait)
        lines = []
        for line in p.stdout:
            lines.append(line)
            if then and "waiting for xab's phase 14" in line:
                self.fabric.cores["xaa"].store.update(then)
                then = None
        p.wait(timeout=240)
        out = "".join(lines)
        self.assertIsNone(then, "xaa never said it was waiting for xab:\n" + out)
        return p.returncode, out, self.checks(ev)

    def evidence_text(self):
        text = []
        for root, _, files in os.walk(os.path.join(self.tmp, "ev-xaa")):
            for f in files:
                text.append(Path(root, f).read_text(errors="replace"))
        return "\n".join(text)

    def test_derived_a_rerun_within_twice_the_wait_is_judged_by_the_stamp_whose_sample_matched(self):
        # xab's run 1 stamped OLD and its sample write failed; run 2 (both within 2W)
        # then stamped NEW and wrote NEW's sample. xaa reads OLD's stamp first. The
        # challenge is the one whose sample matched, so nothing FAILs on other bytes
        # and no row or result carries OLD.
        before, then = {}, {}
        for name in self.xab_names():
            before.update(self.xab_keys(name, self.OLD_CHALLENGE, ago=5, sample=False, persist=False))
            then.update(self.xab_keys(name, self.NEW_CHALLENGE, ago=2))
        rc, out, rows = self.lone_xaa(before, then)
        for name in self.xab_names():
            for row in ("delivery/inbound/release/" + name, "persistence/inbound/release/" + name):
                self.assertEqual(rows[row]["verdict"], "PASS", (row, rows[row], out))
        self.assertNotIn(self.OLD_CHALLENGE, out)
        self.assertNotIn(self.OLD_CHALLENGE, self.evidence_text())

    def test_derived_two_runs_stamps_within_twice_the_wait_end_blocked_never_failed(self):
        # An earlier run of xab's, within 2W, left both its stamp and its sample on
        # the first set; the second set holds another run's stamp and sample. The
        # first set cannot be told from this run's (§6.3); the second, carrying
        # another challenge than the first, is BLOCKED as two runs' stamps, never a
        # byte FAIL, and so is its second sample.
        first, second = self.xab_names()[0], self.xab_names()[1]
        before = self.xab_keys(first, self.OLD_CHALLENGE, ago=5)
        before.update(self.xab_keys(second, self.NEW_CHALLENGE, ago=2))
        rc, out, rows = self.lone_xaa(before, wait="4")
        self.assertEqual(rows["delivery/inbound/release/" + first]["verdict"], "PASS", out)
        for row in ("delivery/inbound/release/" + second, "persistence/inbound/release/" + second):
            self.assertEqual(rows[row]["verdict"], "BLOCKED", (row, rows[row], out))
            self.assertIn("two of xab's runs", rows[row]["reason"], (row, out))
        self.assertNotIn("FAIL", [r["verdict"] for n, r in rows.items() if n.endswith("inbound/release/" + second)])

    def test_derived_an_unstamped_sample_waits_for_a_stamp_in_flight(self):
        # The first run after xab upgrades: its earlier run (an older runner) left an
        # unstamped sample; this run's stamp arrives after xaa first reads. xaa waits
        # instead of learning the earlier sample's challenge, then judges this run's.
        before, then = {}, {}
        for name in self.xab_names():
            before.update(self.xab_keys(name, self.OLD_CHALLENGE))
            then.update(self.xab_keys(name, self.NEW_CHALLENGE, ago=0))
        rc, out, rows = self.lone_xaa(before, then)
        for name in self.xab_names():
            self.assertEqual(rows["delivery/inbound/release/" + name]["verdict"], "PASS", (name, out))
        self.assertNotIn(self.OLD_CHALLENGE, out)
        self.assertNotIn(self.OLD_CHALLENGE, self.evidence_text())

    def test_derived_an_unstamped_sample_is_never_learned_once_the_partner_has_stamped(self):
        # xab's first set carries this run's stamp and sample; the second holds only
        # an unstamped sample (an older runner's, left earlier). xab stamps its runs,
        # so the unstamped one is never learned: BLOCKED, not a FAIL on other bytes.
        # The first set's stamp is dated when xaa first reads it (`ago`): xaa has
        # started by then, so the stamp is this run's however long xaa took to start.
        first, second = self.xab_names()[0], self.xab_names()[1]
        before = self.xab_keys(first, self.NEW_CHALLENGE, ago=0)
        before.update(self.xab_keys(second, self.OLD_CHALLENGE))
        rc, out, rows = self.lone_xaa(before, wait="4")
        self.assertEqual(rows["delivery/inbound/release/" + first]["verdict"], "PASS", out)
        row = rows["delivery/inbound/release/" + second]
        self.assertEqual(row["verdict"], "BLOCKED", (row, out))
        self.assertIn("no run stamp", row["reason"], out)

    def test_derived_a_partner_that_never_stamps_is_read_after_one_wait(self):
        # An older runner stages no stamp at all: its sample is taken once the first
        # set's wait ends with no stamp seen, and the next set is read at once.
        before = {}
        for name in self.xab_names():
            before.update(self.xab_keys(name, self.NEW_CHALLENGE))
        rc, out, rows = self.lone_xaa(before, wait="4")
        for name in self.xab_names():
            self.assertEqual(rows["delivery/inbound/release/" + name]["verdict"], "PASS", (name, out))

    def test_derived_a_stampless_partner_s_second_challenge_is_two_runs_not_a_byte_fail(self):
        # A partner that never stamps: the first set's sample teaches NEW after one
        # wait; the second set holds an earlier run's unstamped sample (OLD). It is
        # BLOCKED as two runs, not a FAIL on other bytes, and the learned row says its
        # run identity is not proven.
        first, second = self.xab_names()[0], self.xab_names()[1]
        before = self.xab_keys(first, self.NEW_CHALLENGE)
        before.update(self.xab_keys(second, self.OLD_CHALLENGE))
        rc, out, rows = self.lone_xaa(before, wait="4")
        row1 = rows["delivery/inbound/release/" + first]
        self.assertEqual(row1["verdict"], "PASS", out)
        self.assertEqual(row1["observations"].get("run_identity"), "unstamped: run identity not proven", row1)
        for row in ("delivery/inbound/release/" + second, "persistence/inbound/release/" + second):
            self.assertEqual(rows[row]["verdict"], "BLOCKED", (row, rows[row], out))
            self.assertIn("two of xab's runs", rows[row]["reason"], (row, out))

    def test_derived_a_stamp_s_own_sample_read_at_the_end_of_the_wait_is_this_run_s(self):
        # xab's stamp is there from the start; every read of its sample inside the
        # loop fails, and the read at the end of the wait finds the stamp's own sample.
        # That is this run's sample, not "an earlier run's".
        before = {}
        for name in self.xab_names():
            before.update(self.xab_keys(name, self.NEW_CHALLENGE, ago=0))
        state = os.path.join(self.tmp, "last-read")
        real = shutil.which("curl")
        flaky = self.stand_in("curl", (
            'put=0; url=""\n'
            'for a in "$@"; do [ "$a" = PUT ] && put=1; case "$a" in http*) url="$a";; esac; done\n'
            'if [ "$put" = 0 ] && [[ "$url" == */run ]]; then echo stamp > %s; fi\n'
            'if [ "$put" = 0 ] && [[ "$url" =~ /acceptance/[^/]+/xab$ ]]; then\n'
            '  if [ "$(cat %s 2>/dev/null)" = stamp ]; then echo sample > %s; exit 7; fi\n'
            'fi\n'
            'exec %s "$@"\n') % (state, state, state, real))
        self.fabric.faults.stale["xaa"] = before
        ev = os.path.join(self.tmp, "ev-xaa")
        p = self.derived_side("xaa", ev, wait="4", extra=flaky)
        out = p.communicate(timeout=240)[0]
        rows = self.checks(ev)
        for name in self.xab_names():
            row = rows["delivery/inbound/release/" + name]
            self.assertEqual(row["verdict"], "PASS", (name, row, out))

    def test_derived_a_garbled_stamp_counts_as_a_stamp_seen(self):
        # The first set's stamp is garbled; the second holds only an unstamped earlier
        # sample. xab's runner stamps (badly), so the unstamped sample is never learned.
        first, second = self.xab_names()[0], self.xab_names()[1]
        wire = "from/xab/%s/acceptance/%s/xab" % (first, self.session_id())
        before = {wire + "/run": b"xab-not-a-stamp"}
        before.update(self.xab_keys(second, self.OLD_CHALLENGE))
        rc, out, rows = self.lone_xaa(before, wait="4")
        row = rows["delivery/inbound/release/" + second]
        self.assertEqual(row["verdict"], "BLOCKED", (row, out))
        self.assertIn("no run stamp", row["reason"], out)
        # The earlier sample's bytes are recorded as what was read, never learned as
        # xab's challenge.
        self.assertIn(self.OLD_CHALLENGE, row["observations"]["received"])
        self.assertNotIn(self.OLD_CHALLENGE, row["observations"]["sent_expected"])
        self.assertNotIn(self.OLD_CHALLENGE, out)

    def test_derived_clocks_apart_by_less_than_the_wait_keep_a_fresh_sample(self):
        # xaa's clock runs 8 s behind, the wait is 10 s, and xab starts 3 s after xaa:
        # the runs overlap and the clocks agree within the wait, so xab keeps xaa's
        # sample (its start is within twice the wait of xab's).
        real = shutil.which("date")
        skew = self.stand_in("date", (
            'if [ "$*" = "-u +%%s" ]; then echo $(( $(%s -u +%%s) - 8 )); exit 0; fi\n'
            'exec %s "$@"\n') % (real, real))
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived(delay=3, wait="10", extra=skew, second={"extra": None})
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        self.assertNotIn("from an earlier run", out_b)
        for side in ("xaa", "xab"):
            self.assertEqual(self.bilateral(ev[side])["verdict"], "PASS", side)

    def test_derived_a_signature_over_other_bytes_than_the_published_result_is_refused_gb_272(self):
        # The partner's signer signs other bytes than the result it publishes; a wire
        # edit of the value on the core is not exercised here.
        # Only the receiver's refusal is asserted; the peer's result is discarded.
        ev, (rc_a, out_a), _ = self.both_derived(second={"extra": {"FED_TEST_SIG_TAMPER": "1"}})
        self.assertEqual(rc_a, 2, out_a)
        self.assertIn("GB-272", out_a)
        self.assertFalse(os.path.isdir(os.path.join(ev["xaa"], "bilateral")), out_a)

    # --- own surfaces only: each side observes its own refusals and SPIRE (§6.4) ----
    OWN_ROWS = ("refusal/control-allowed", "refusal/out-of-lane-denied", "refusal/spoof-denied",
                "refusal/forged-name", "refusal/impersonation", "spire/refresh", "spire/bundle-held",
                "spire/svid-chains")

    def rows_of(self, evidence):
        with open(os.path.join(evidence, "result.json")) as f:
            return {c["name"]: c for c in json.load(f)["checks"]}

    def test_own_surfaces_only_both_sides_reach_a_combined_pass(self):
        ev, (rc_a, out_a), (rc_b, out_b) = self.both_derived(own=True)
        for side, rc, out in (("xaa", rc_a, out_a), ("xab", rc_b, out_b)):
            combined = self.bilateral(ev[side])
            self.assertEqual((rc, combined["verdict"]), (0, "PASS"), (side, out, combined["open"], combined["failed"]))
            rows = self.rows_of(ev[side])
            for name in self.OWN_ROWS:
                self.assertEqual(rows[name]["verdict"], "PASS", (side, name, out))
            with open(os.path.join(ev[side], "result.json")) as f:
                inputs = json.load(f)["inputs"]
            for k in ("theirs_rest", "ssh_theirs", "probe_launcher", "spire_theirs"):
                self.assertFalse(inputs.get(k), (side, k, inputs))
            self.assertTrue(inputs.get("probe_launcher_ours"), inputs)

    def test_own_surfaces_the_partners_probe_ids_are_read_only_from_its_lane(self):
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        self.assertEqual(rc_a, 0, out_a)
        key = self.rows_of(ev["xaa"])["refusal/control-allowed"]["observations"]["probes_key"]
        self.assertTrue(key.startswith("from/xab/"), key)
        self.assertIn("/acceptance/ses-", key)

    def test_own_surfaces_no_staged_value_is_key_material(self):
        # The fake spire-server's `x509 mint` prints the workload's private key after its
        # certificate, as the real one does: only the certificate may be staged. Every
        # value either side put on a core is checked, raw and base64-decoded: no private
        # key block, no signing seed, and none of the fake fabric's private keys.
        import base64, re
        _, (rc_a, out_a), (rc_b, out_b) = self.both_derived(own=True)
        self.assertEqual((rc_a, rc_b), (0, 0), (out_a, out_b))
        keys = [k for _, k, _ in self.fabric.puts]
        for want in ("/probes", "/spire-bundle", "/spire-svid"):
            self.assertTrue(any(k.endswith(want) for k in keys), (want, keys))
        tk = Path(HERE).parents[1] / "federation/vectors/testkeys"
        secrets = [(tk / ("%s.seed.hex" % s)).read_text().strip().encode() for s in ("xaa", "xab")]
        spire = os.path.join(self.tmp, "spire")
        for fn in os.listdir(spire):
            if fn.endswith(".key"):
                with open(os.path.join(spire, fn), "rb") as kf:
                    body = b"".join(l.strip() for l in kf if not l.startswith(b"-----"))
                secrets.append(body[:40])

        def texts(v):
            yield v
            for tok in re.findall(rb"[A-Za-z0-9+/=]{16,}", v):
                try:
                    yield base64.b64decode(tok + b"=" * (-len(tok) % 4), validate=True)
                except Exception:
                    pass  # not base64 after all: the raw value was checked above
        for side, k, v in self.fabric.puts:
            for t in texts(v):
                self.assertNotIn(b"PRIVATE KEY", t, (side, k))
                for sec in secrets:
                    self.assertNotIn(sec, t, (side, k))
            if k.endswith(("/probes", "/spire-bundle", "/spire-svid")):
                # Ids and certificates only: no 32-byte value in hex or base64 beside them.
                self.assertIsNone(re.search(rb"(?<![0-9a-f])[0-9a-f]{64}(?![0-9a-f])", v), (side, k, v[:80]))

    def test_own_surfaces_without_the_partners_probes_our_refusal_rows_are_blocked_naming_them(self):
        ev, (rc_a, out_a), _ = self.both_derived(own=True, second={"own": False})
        rows = self.rows_of(ev["xaa"])
        self.assertEqual(rows["refusal/acl-trace"]["verdict"], "BLOCKED", out_a)
        self.assertIn("xab", rows["refusal/acl-trace"]["reason"])
        self.assertNotEqual(rc_a, 0, out_a)

    def test_own_surfaces_probe_ids_from_an_earlier_run_teach_nothing(self):
        # xab's probe ids from another run of this session sit on our core; xab runs now
        # without a launcher. The stale ids carry the other run's challenge: not used.
        st = "all"
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        out = os.path.join(self.tmp, "derived.json")
        subprocess.run(["python3", str(HERE / "acceptance_session.py"), "derive", "--agreement", path,
                        "--us", "xaa", "--attempt", "1", "--out", out], capture_output=True, check=True)
        with open(out) as f:
            sid = json.load(f)["session_id"]
        self.fabric.faults.stale["xaa"] = {
            "from/xab/%s/acceptance/%s/xab/probes" % (st, sid): ("%s agreed=%s forged=%s impersonate=%s" % (
                "0" * 32, self.fabric.agreed_zid, self.fabric.zids["forged"], self.fabric.zids["impersonate"])).encode()}
        ev, (rc_a, out_a), _ = self.both_derived(own=True, second={"own": False})
        rows = self.rows_of(ev["xaa"])
        for name in ("refusal/acl-trace",):
            self.assertEqual(rows[name]["verdict"], "BLOCKED", (name, out_a))
        self.assertNotIn("refusal/control-allowed", [n for n, c in rows.items() if c["verdict"] == "PASS"], out_a)

    def test_own_surfaces_a_forged_probe_our_edge_authorizes_fails_our_row(self):
        self.fabric.faults.authorize_forged = True
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        self.assertEqual(self.rows_of(ev["xaa"])["refusal/forged-name"]["verdict"], "FAIL", out_a)
        self.assertEqual(rc_a, 1, out_a)

    def test_own_surfaces_an_svid_their_server_got_from_our_root_fails(self):
        self.fabric.faults.svid_from_other_ca.add("xab")
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        self.assertEqual(self.rows_of(ev["xaa"])["spire/svid-chains"]["verdict"], "FAIL", out_a)

    def test_both_probe_launchers_at_once_are_refused(self):
        path = os.path.join(self.tmp, "agreement.json")
        with open(path, "w") as f:
            json.dump(self.ag, f)
        env = self.derived_env("xaa", os.path.join(self.tmp, "ev-both"), own=True,
                               extra={"FED_PROBE_LAUNCHER": "/bin/false"})
        p = subprocess.run(["bash", str(SCRIPT), path, "--as", "xaa"], env=env, capture_output=True, text=True,
                           timeout=60)
        self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
        self.assertIn("FED_PROBE_LAUNCHER_OURS", p.stdout + p.stderr)

    def old_line(self, side, zid):
        """A line an earlier run left in `side`'s edge log for `zid`: a day old."""
        import time
        with open(self.fabric.trace_paths[side], "a") as f:
            f.write("%s INFO zenoh::net::routing::interceptor::access_control: ACL: peer %s did not match any "
                    "configured ACL subject\n" % (fakefabric.stamp(time.time() - 86400), zid))

    def staged_probes(self, side):
        vals = [v.decode() for s, k, v in self.fabric.puts if s == side and k.endswith("/probes")]
        return vals[-1] if vals else ""

    def test_own_surfaces_a_reused_zid_with_an_earlier_runs_line_is_never_a_pass(self):
        # xab's launcher reuses fixed zids; this run its forged session reaches xaa's edge
        # but the edge records nothing for it. A day-old unmatched line for the same zid is
        # in xaa's log: it is not this run's, so the row is BLOCKED, not PASS.
        self.fabric.faults.silent_probes.add("forged")
        self.old_line("xaa", self.fabric.zids["forged"])
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        row = self.rows_of(ev["xaa"])["refusal/forged-name"]
        self.assertIn(row["verdict"], ("BLOCKED", "FAIL"), out_a)
        self.assertIn("since xab's run started", row["reason"], out_a)
        self.assertNotEqual(rc_a, 0, out_a)

    def edge_lines(self, side, *lines):
        with open(self.fabric.trace_paths[side], "a") as f:
            for line in lines:
                f.write(line + "\n")

    def test_own_surfaces_a_garbled_timestamp_never_hides_a_later_authorization(self):
        # The edge records nothing itself for xab's forged session this run; its log holds,
        # stamped after xab's start: a valid unmatched line, a line with an ISO-shaped but
        # impossible stamp, then an authorization. The authorization must not be lost.
        import time
        self.fabric.faults.silent_probes.add("forged")
        z = self.fabric.zids["forged"]
        later = fakefabric.stamp(time.time() + 60)
        self.edge_lines("xaa",
                        "%s INFO access_control: ACL: peer %s did not match any configured ACL subject" % (later, z),
                        "2026-13-45T99:99:99Z INFO access_control: %s on Interface(eth0) garbled" % z,
                        "%s INFO access_control: %s on forged is authorized to Put (ingress) on from/xab/x" % (later, z))
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        row = self.rows_of(ev["xaa"])["refusal/forged-name"]
        self.assertEqual(row["verdict"], "FAIL", out_a)

    def test_own_surfaces_an_unreadable_edge_log_blocks_the_row(self):
        # A copy of the runner whose log filter cannot open the log (the seam): the
        # row is BLOCKED, never PASS.
        d = os.path.join(self.tmp, "runner")
        shutil.copytree(HERE, d, ignore=shutil.ignore_patterns("__pycache__", "test_*"))
        with open(os.path.join(d, "acceptance.sh")) as f:
            src = f.read()
        cut = 'python3 - "$TRACE" "$1" "$PEER_STARTED" "$RUN_START_SLACK" <<\'PY\''
        self.assertIn(cut, src)
        with open(os.path.join(d, "acceptance.sh"), "w") as f:
            f.write(src.replace(cut, 'python3 - /nonexistent "$1" "$PEER_STARTED" "$RUN_START_SLACK" <<\'PY\'', 1))
        ev, (rc_a, out_a), _ = self.both_derived(own=True, script=os.path.join(d, "acceptance.sh"),
                                                 second={"script": None})
        rows = self.rows_of(ev["xaa"])
        for name in ("refusal/forged-name", "refusal/impersonation"):
            self.assertEqual(rows[name]["verdict"], "BLOCKED", (name, out_a))
            self.assertIn("could not be read", rows[name]["reason"])

    def test_own_surfaces_an_edge_clock_20s_behind_the_partners_still_passes(self):
        # xaa's edge stamps its lines 20 s behind real time, so xab's (honest) connect
        # lines read as before xab's start: within the run-start slack, they count.
        self.fabric.faults.trace_clock_offset["xaa"] = -20
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        rows = self.rows_of(ev["xaa"])
        for name in ("refusal/forged-name", "refusal/impersonation"):
            self.assertEqual(rows[name]["verdict"], "PASS", (name, out_a))

    def test_own_surfaces_an_edge_clock_far_behind_blocks_the_rows(self):
        # Two minutes behind is past the slack: the lines cannot be tied to this run.
        self.fabric.faults.trace_clock_offset["xaa"] = -120
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        rows = self.rows_of(ev["xaa"])
        for name in ("refusal/forged-name", "refusal/impersonation"):
            self.assertEqual(rows[name]["verdict"], "BLOCKED", (name, out_a))

    def test_own_surfaces_a_probe_session_that_does_not_answer_is_not_staged(self):
        # xab's forged session does not answer its PUT: its id is not staged, and xaa's
        # row is BLOCKED naming xab's launcher, even with an earlier run's line in its log.
        self.fabric.faults.own_dead["xab"] = {"forged"}
        self.old_line("xaa", self.fabric.zids["forged"])
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        staged = self.staged_probes("xab")
        self.assertIn("agreed=", staged)
        self.assertNotIn("forged=", staged)
        row = self.rows_of(ev["xaa"])["refusal/forged-name"]
        self.assertEqual(row["verdict"], "BLOCKED", out_a)
        self.assertIn("xab", row["reason"])
        self.assertIn("FED_PROBE_LAUNCHER_OURS", row["reason"] + row.get("observations", {}).get("hint", "") + out_a)

    def test_own_surfaces_an_agreed_probe_that_does_not_answer_blocks_the_control_naming_them(self):
        self.fabric.faults.own_dead["xab"] = {"agreed"}
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        self.assertNotIn("agreed=", self.staged_probes("xab"))
        row = self.rows_of(ev["xaa"])["refusal/control-allowed"]
        self.assertEqual(row["verdict"], "BLOCKED", out_a)
        self.assertIn("xab", row["reason"])
        self.assertIn("FED_PROBE_LAUNCHER_OURS", row["reason"])
        self.assertNotIn("the control did not land", out_a)

    def test_own_surfaces_one_zid_staged_for_two_kinds_is_refused(self):
        z = self.fabric.zids["forged"]
        self.fabric.faults.own_zid["xab"] = {"impersonate": z}
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        rows = self.rows_of(ev["xaa"])
        self.assertEqual(rows["refusal/acl-trace"]["verdict"], "BLOCKED", out_a)
        self.assertIn("same zenoh id", rows["refusal/acl-trace"]["reason"])
        for name in ("refusal/forged-name", "refusal/impersonation", "refusal/control-allowed"):
            self.assertNotEqual(rows.get(name, {}).get("verdict"), "PASS", (name, out_a))

    def test_own_surfaces_the_staged_probe_ids_carry_the_runs_start(self):
        ev, (rc_a, out_a), _ = self.both_derived(own=True)
        self.assertEqual(rc_a, 0, out_a)
        self.assertRegex(self.staged_probes("xab"), r" started=[0-9]{10}( |$)")

    def test_own_surfaces_a_staged_value_holding_key_material_stops_the_run(self):
        # A copy of the runner whose certificate filter lets the minted key through (the
        # seam): the guard after it must stop the run, exit 1, with nothing staged.
        d = os.path.join(self.tmp, "runner")
        shutil.copytree(HERE, d, ignore=shutil.ignore_patterns("__pycache__", "test_*"))
        with open(os.path.join(d, "acceptance.sh")) as f:
            src = f.read()
        cut = "      | awk '/-----BEGIN CERTIFICATE-----/{f=1} f{print} /-----END CERTIFICATE-----/{if(f)exit}')"
        self.assertIn(cut, src)
        with open(os.path.join(d, "acceptance.sh"), "w") as f:
            f.write(src.replace(cut, "      | cat)", 1))
        ev, (rc_a, out_a), _ = self.both_derived(own=True, script=os.path.join(d, "acceptance.sh"),
                                                 second={"script": None})
        self.assertEqual(rc_a, 1, out_a)
        self.assertIn("private key material", out_a)
        for side, k, v in self.fabric.puts:
            if side == "xaa":
                self.assertNotIn(b"PRIVATE KEY", v, k)
        self.assertFalse([k for s, k, _ in self.fabric.puts if s == "xaa" and k.endswith("/spire-svid")])

    def test_our_own_launcher_outside_a_session_is_refused(self):
        rc, out = self.run_acceptance(extra={"FED_PROBE_LAUNCHER_OURS": "/bin/false"})
        self.assertEqual(rc, 2, out)
        self.assertIn("FED_PROBE_LAUNCHER_OURS", out)
        self.assertIn("bilateral session", out)

    def test_derived_mode_refuses_without_a_signer_or_a_verifier(self):
        for missing in ("FED_ACCEPTANCE_SIGNER", "FED_ACCEPTANCE_VERIFIER"):
            path = os.path.join(self.tmp, "agreement.json")
            with open(path, "w") as f:
                json.dump(self.ag, f)
            env = self.derived_env("xaa", os.path.join(self.tmp, "ev-" + missing))
            env.pop(missing)
            p = subprocess.run(["bash", str(SCRIPT), path, "--as", "xaa"], env=env, capture_output=True, text=True,
                               timeout=60)
            self.assertEqual(p.returncode, 2, p.stdout + p.stderr)
            self.assertIn(missing, p.stderr)
            self.assertIn("no result is published unsigned", p.stderr)

    def test_a_manifest_for_another_agreement_stops_the_run(self):
        other = os.path.join(self.tmp, "other.json")
        with open(other, "w") as f:
            json.dump(fakefabric.agreement(them="xac"), f)
        manifest = os.path.join(self.tmp, "session.json")
        subprocess.run(["python3", str(HERE / "acceptance_session.py"), "new", "--agreement", other, "--us", "xaa",
                        "--out", manifest], check=True, capture_output=True)
        rc, out = self.run_acceptance(extra={"FED_ACCEPTANCE_SESSION": manifest})
        self.assertEqual(rc, 2, out)
        self.assertIn("another agreement", out)

    # --- run-owned, time-limited probes ---------------------------------------
    def launcher(self, fab_env, stuck=False, alive_cmd=None, stop_cmd=None):
        """A launcher in the contract acceptance.sh documents. Its 'sessions' are files;
        the fabric already serves the three probe surfaces."""
        state = os.path.join(self.tmp, "launched")
        os.makedirs(state, exist_ok=True)
        path = os.path.join(self.tmp, "launcher.sh")
        rest = {"agreed": fab_env["FED_PROBE_THEIRS_REST"], "forged": fab_env["FED_PROBE_FORGED_REST"],
                "impersonate": fab_env["FED_PROBE_IMPERSONATE_REST"]}
        zid = {"agreed": self.fabric.agreed_zid, "forged": self.fabric.zids["forged"],
               "impersonate": self.fabric.zids["impersonate"]}
        with open(path, "w") as f:
            f.write("#!/bin/sh\nS='%s'\n" % state)
            f.write('case "$1" in\n  start)\n    case "$2" in\n')
            for k in rest:
                f.write('      %s) echo "REST=%s"; echo "ZID=%s" ;;\n' % (k, rest[k], zid[k]))
            f.write('    esac\n    echo "HANDLE=$2-$3"; echo "$4" > "$S/$2-$3" ;;\n')
            f.write('  stop) %s ;;\n' % (stop_cmd or ("true" if stuck else 'rm -f "$S/$2"')))
            f.write('  alive) %s ;;\nesac\n' % (alive_cmd or 'test -f "$S/$2"'))
        os.chmod(path, 0o700)
        return path, state

    def test_probes_are_started_by_the_run_and_cleaned_up(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab)
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_FORGED_ZID",
                  "FED_PROBE_IMPERSONATE_REST", "FED_PROBE_IMPERSONATE_ZID"],
            extra={"FED_PROBE_LAUNCHER": launcher, "FED_PROBE_TTL": "60"})
        v = self.verdicts()
        self.assertEqual(rc, 0, out)
        for name in ("refusal/control-allowed", "refusal/forged-name", "refusal/impersonation", "cleanup/probes"):
            self.assertEqual(v.get(name), "PASS", (name, out))
        self.assertEqual(os.listdir(state), [], "a probe is still running")
        run = self.bundle()["run"]["id"]
        m = json.load(open(os.path.join(self.tmp, "evidence", "acceptance", "probes", run + ".json")))
        self.assertEqual(sorted(r["kind"] for r in m["resources"].values()), ["agreed", "forged", "impersonate"])
        self.assertEqual({r["status"] for r in m["resources"].values()}, {"gone"})
        self.assertTrue(all(r["deadline"] > 0 for r in m["resources"].values()))

    def probe_manifest(self):
        run = self.bundle()["run"]["id"]
        return json.load(open(os.path.join(self.tmp, "evidence", "acceptance", "probes", run + ".json")))

    def test_a_launcher_that_verifies_each_key_deleted_has_it_recorded(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab, stop_cmd='rm -f "$S/$2"; echo KEY_DELETED=verified')
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.verdicts()["cleanup/probes"], "PASS", out)
        self.assertEqual({r["key_deleted"] for r in self.probe_manifest()["resources"].values()}, {"verified"})
        c = [c for c in self.bundle()["checks"] if c["name"] == "cleanup/probes"][0]
        self.assertIn("verified", json.dumps(c), c)

    def test_a_key_the_launcher_could_not_delete_fails_cleanup(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab, stop_cmd='rm -f "$S/$2"; echo KEY_DELETED=failed; exit 1')
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 1, out)
        c = [c for c in self.bundle()["checks"] if c["name"] == "cleanup/probes"][0]
        self.assertEqual((c["verdict"], c["required"]), ("FAIL", True))
        self.assertEqual({r["key_deleted"] for r in self.probe_manifest()["resources"].values()}, {"failed"})

    def test_a_launcher_that_reports_nothing_about_keys_is_recorded_as_such(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab)
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 0, out)
        self.assertEqual({r["key_deleted"] for r in self.probe_manifest()["resources"].values()}, {"not reported"})

    def test_incomplete_cleanup_is_a_required_failure(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab, stuck=True)
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 1, out)
        c = [c for c in self.bundle()["checks"] if c["name"] == "cleanup/probes"][0]
        self.assertEqual((c["verdict"], c["required"]), ("FAIL", True))

    def test_an_interrupted_runs_probes_are_cleaned_first_and_nothing_else(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab)
        probes = os.path.join(self.tmp, "evidence", "acceptance", "probes")
        os.makedirs(probes)
        # A run killed before its cleanup left one probe; an unrelated session is not ours.
        open(os.path.join(state, "agreed-acc-old"), "w").close()
        open(os.path.join(state, "somebody-elses"), "w").close()
        with open(os.path.join(probes, "acc-old.json"), "w") as f:
            json.dump({"run": "acc-old", "resources": {"agreed-acc-old": {"kind": "agreed", "status": "started",
                                                                          "deadline": 1}}}, f)
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.verdicts()["cleanup/earlier-runs"], "PASS", out)
        self.assertEqual(os.listdir(state), ["somebody-elses"])

    def test_an_unreadable_earlier_manifest_is_not_taken_as_clean(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab)
        probes = os.path.join(self.tmp, "evidence", "acceptance", "probes")
        os.makedirs(probes)
        with open(os.path.join(probes, "acc-broken.json"), "w") as f:
            f.write("{not json")
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["cleanup/earlier-runs"], "FAIL", out)

    def test_only_exit_1_from_alive_means_gone(self):
        # A launcher whose `alive` errors (exit 2) cannot show a probe is gone.
        fab = self.fabric.start()
        launcher, state = self.launcher(fab, alive_cmd="exit 2")
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.verdicts()["cleanup/probes"], "FAIL", out)

    def earlier_manifest(self, **fields):
        probes = os.path.join(self.tmp, "evidence", "acceptance", "probes")
        os.makedirs(probes, exist_ok=True)
        m = {"run": "acc-old", "resources": {"agreed-acc-old": {"kind": "agreed", "status": "started", "deadline": 1}}}
        m.update(fields)
        with open(os.path.join(probes, "acc-old.json"), "w") as f:
            json.dump(m, f)

    def test_a_reused_pid_does_not_hide_a_dead_runs_probes(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab)
        open(os.path.join(state, "agreed-acc-old"), "w").close()
        # This test process is alive, but it is not the run that wrote the manifest.
        self.earlier_manifest(pid=os.getpid(), pid_start="Thu Jan  1 00:00:00 1970")
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(self.verdicts()["cleanup/earlier-runs"], "PASS", out)
        self.assertEqual(os.listdir(state), [])

    def test_a_live_pid_without_a_start_time_is_blocked_not_skipped(self):
        fab = self.fabric.start()
        launcher, state = self.launcher(fab)
        open(os.path.join(state, "agreed-acc-old"), "w").close()
        self.earlier_manifest(pid=os.getpid())
        rc, out = self.run_acceptance(
            drop=["FED_PROBE_THEIRS_REST", "FED_PROBE_FORGED_REST", "FED_PROBE_IMPERSONATE_REST"],
            extra={"FED_PROBE_LAUNCHER": launcher})
        self.assertEqual(self.verdicts()["cleanup/earlier-runs-unknown"], "BLOCKED", out)
        self.assertEqual(rc, 3, out)
        self.assertIn("agreed-acc-old", os.listdir(state))

    def test_without_a_launcher_nothing_is_started_or_cleaned(self):
        rc, out = self.run_acceptance()
        self.assertEqual(rc, 0, out)
        self.assertFalse([n for n in self.verdicts() if n.startswith("cleanup/")], out)
        self.assertFalse(self.bundle()["inputs"]["probe_launcher"])

    def test_a_broken_round_trip_fails(self):
        self.fabric.faults.responder_down.add("xab")
        rc, out = self.run_acceptance(extra={})
        self.assertEqual(rc, 1, out)
        self.assertIn("did not complete", out)

    def test_a_dropped_round_trip_probe_is_sent_again_after_the_gap(self):
        # In a session both runners send at once, so the partner's edge can drop our
        # probe inside its downsampling interval; the next probe, one gap later, completes.
        self.fabric.faults.drop_probes["xab"] = 1
        rc, out = self.run_acceptance(extra={"FED_ACCEPTANCE_GAP": "0.8"})
        self.assertEqual(rc, 0, out)
        self.assertEqual(self.verdicts()["roundtrip/outbound/release/all"], "PASS", out)
        times = [t for t, side, key in self.fabric.probe_log if side == "xaa" and key.startswith("release/all/fedcheck/")]
        self.assertEqual(len(times), 2, out)
        self.assertGreaterEqual(times[1] - times[0], 0.8 - 0.05, out)


if __name__ == "__main__":
    unittest.main()
