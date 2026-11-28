#!/usr/bin/env python3
"""Standup phase 11's driver (ops/scripts/standup-bootstrap-token.py, ADR 1274).

The management host is a stand-in `ssh` on PATH that plays the host's side:
docker inspect of management-a, its compose labels, the image's repo digests,
and the fleet image's `service-user` commands over a pending-token file. It
records every argv it is given. The NetBird API check is patched. Each test
checks what reached the host and the store, and that the token never appears
in the driver's output or in any argv.
"""
import base64
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("standup_bootstrap_token", ROOT / "ops/scripts/standup-bootstrap-token.py")
drv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drv)

USER = "svc-zagros-netbird-infra"
ENV = "ZAGROS_NB_API_TOKEN"

HOST = r'''#!/usr/bin/env python3
import json, os, random, string, sys
state_dir = os.environ["SU_STATE"]
st_path = os.path.join(state_dir, "state.json")
st = json.load(open(st_path))
remote = sys.argv[-1]
with open(os.path.join(state_dir, "argv.log"), "a") as f:
    f.write(json.dumps(sys.argv) + "\n")
def save():
    json.dump(st, open(st_path, "w"))
RUNNING = "sha256:" + "a" * 64
BASE = ("sudo docker compose -p mgmt-host --project-directory /opt/mgmt-host "
        "-f /opt/mgmt-host/docker-compose.yml -f /opt/mgmt-host/docker-compose.phase2.yml")
ONESHOT = BASE + " run --rm --no-deps --pull never -T -v /run/goat-bootstrap:/run/goat-bootstrap management-a service-user "
if "--format '{{.Image}}'" in remote:
    print(RUNNING); sys.exit(0)
if remote == BASE + " config --format json":
    print(json.dumps({"services": {"management-a": {"image": st["declared"]}}})); sys.exit(0)
if remote == "sudo docker image inspect " + st["declared"] + " --format '{{.Id}}'":
    print(st.get("declared_id", RUNNING)); sys.exit(0)
if remote == "sudo docker image inspect " + RUNNING + " --format '{{json .RepoDigests}}'":
    print(json.dumps([st["repo_digest"]])); sys.exit(0)
if 'com.docker.compose.project.working_dir' in remote:
    print("/opt/mgmt-host"); sys.exit(0)
if 'com.docker.compose.project.config_files' in remote:
    print("/opt/mgmt-host/docker-compose.yml,/opt/mgmt-host/docker-compose.phase2.yml"); sys.exit(0)
if 'com.docker.compose.project"' in remote:
    print("mgmt-host"); sys.exit(0)
if remote == BASE + " run --rm --no-deps --pull never -T management-a service-user bootstrap-user --help":
    if st.get("old_image"):
        # An image built before ADR 1274: the old command, no bootstrap-user.
        print("Manage service users offline\n\nUsage:\n  management service-user [command]\n\nAvailable Commands:\n  bootstrap-token"); sys.exit(0)
    print("Usage:\n  management service-user bootstrap-user [flags]\n\nFlags:\n      --dir string   root-owned 0700 tmpfs directory"); sys.exit(0)
if "service-user bootstrap-user" in remote:
    # The one-shot exactly as the ADR shapes it: no other service started, no
    # pull, no tty, only the tmpfs directory mounted, made root 0700 first.
    want = "sudo install -d -m 0700 -o root -g root /run/goat-bootstrap && " + ONESHOT + "bootstrap-user "
    if not remote.startswith(want):
        print("one-shot not as declared: " + remote, file=sys.stderr); sys.exit(98)
    st["compose_calls"] = st.get("compose_calls", 0) + 1
    if st.get("rows", 0) >= 2:
        print(json.dumps({"state": "rotating", "user_id": "u1", "name": "n", "token_ids": ["a", "b"]}))
        print("a rotation was not finished; ... bootstrap-token-revoke --name n --token-id a", file=sys.stderr)
        save(); sys.exit(1)
    if st.get("rows", 0) == 1:
        print(json.dumps({"state": "held", "user_id": "u1", "name": "n"})); save(); sys.exit(0)
    if not st.get("pending"):
        st["pending"] = "nbp_" + "".join(random.choice(string.ascii_letters + string.digits) for _ in range(36))
    print(json.dumps({"state": "pending", "user_id": "u1", "name": "n"})); save(); sys.exit(0)
if remote.startswith("sudo cat "):
    if not st.get("pending"):
        print("cat: no such file", file=sys.stderr); sys.exit(1)
    sys.stdout.write(st["pending"]); sys.exit(0)
if "service-user bootstrap-token-install" in remote:
    if not remote.startswith(ONESHOT + "bootstrap-token-install "):
        print("one-shot not as declared: " + remote, file=sys.stderr); sys.exit(98)
    got = sys.stdin.read().rstrip("\n")
    if got != st.get("pending"):
        print("the value on stdin is not the pending token", file=sys.stderr); sys.exit(1)
    st["rows"] = 1; st["installed"] = got; st["pending"] = ""
    print(json.dumps({"state": "installed", "user_id": "u1", "name": "n"})); save(); sys.exit(0)
print("unexpected remote command: " + remote, file=sys.stderr); sys.exit(97)
'''


class Driver(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self._tmp.name)
        self.bin = self.dir / "bin"
        self.bin.mkdir()
        (self.bin / "ssh").write_text(HOST)
        (self.bin / "ssh").chmod(0o755)
        self.state = self.dir / "state"
        self.state.mkdir()
        self.digest = drv.fleet_digest()
        self.host_state({"repo_digest": "ghcr.io/example/netbird-mgmt@" + self.digest,
                         "declared": "ghcr.io/example/netbird-mgmt@" + self.digest})
        self.envfile = self.dir / "s.env"
        self.envfile.write_text("")
        os.chmod(self.envfile, 0o600)
        self.checkout = self.dir / "checkout"
        (self.checkout / ".envrc.local.d").mkdir(parents=True)
        self.gvars = {"site_id": "zagros", "goatnet_founder": True, "public_mgmt_fqdn": "netbird.zagros.example.org"}
        self.auth = {}  # token -> True when it authenticates as USER

    def tearDown(self):
        self._tmp.cleanup()

    def host_state(self, update=None):
        p = self.state / "state.json"
        st = json.loads(p.read_text()) if p.exists() else {}
        if update is not None:
            st.update(update)
            p.write_text(json.dumps(st))
        return st

    def argv_log(self):
        p = self.state / "argv.log"
        return p.read_text() if p.exists() else ""

    def run_driver(self, store="env-file"):
        ids = drv.load_identities()
        env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}", SU_STATE=str(self.state),
                   GOAT_SECRET_STORE=store, GOAT_SECRET_ENV_FILE=str(self.envfile))
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.dict(os.environ, env, clear=True), \
                mock.patch.object(drv, "load_identities", return_value=ids), \
                mock.patch.object(ids, "load_group", return_value=self.gvars), \
                mock.patch.object(ids, "primary_checkout", return_value=self.checkout), \
                mock.patch.object(drv, "authenticates", side_effect=self.authenticates), \
                mock.patch.object(drv, "group_hosts", return_value=["ops@192.0.2.10", "ops@192.0.2.11"]), \
                mock.patch.object(drv, "state_dir", return_value=self.dir / "goat-state"), \
                contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = drv.main(["zagros"] + (self.args if hasattr(self, "args") else []))
        return rc, out.getvalue(), err.getvalue()

    def authenticates(self, _ids, _fqdn, _site, value, user):
        # A token switched on by the stand-in host works; so does one a test
        # marks as working.
        works = self.auth.get(value, False) or value == self.host_state().get("installed")
        return (works and user == USER and not getattr(self, "api_refuses", False)), "stand-in"


    def stored(self):
        text = self.envfile.read_text()
        m = re.search(rf"^{ENV}=['\"]?([^'\"\n]*)", text, re.M)
        return m.group(1) if m else ""

    def assert_no_token(self, token, *texts):
        forms = [token]
        for i in range(3):
            enc = base64.b64encode((("x" * i) + token).encode()).decode()
            forms.append(enc[4:-4])
        for text in texts + (self.argv_log(),):
            self.assertNotIn("nbp_", text)
            for f in forms:
                self.assertNotIn(f, text)

    # -- the end state and the mint ------------------------------------------

    def test_a_founder_mints_stores_reads_back_and_switches_on(self):
        rc, out, err = self.run_driver()
        self.assertEqual(rc, 0, out + err)
        token = self.host_state()["installed"]
        self.assertRegex(token, r"^nbp_[A-Za-z0-9]{36}$")
        self.assertEqual(self.stored(), token, "the store holds what was switched on")
        self.assertIn("read back, switched on, and checked", out)
        self.assert_no_token(token, out, err)

    def test_a_stored_token_that_works_means_nothing_is_done(self):
        self.envfile.write_text(f"{ENV}=nbp_{'w' * 36}\n")
        self.auth[f"nbp_{'w' * 36}"] = True
        rc, out, err = self.run_driver()
        self.assertEqual(rc, 0, out + err)
        self.assertIn("nothing to do", out)
        self.assertEqual(self.argv_log(), "", "the host is never reached")

    def test_a_run_that_stored_and_stopped_resumes(self):
        pending = "nbp_" + "p" * 36
        self.host_state({"pending": pending})
        self.envfile.write_text(f"{ENV}={pending}\n")
        rc, out, err = self.run_driver()
        self.assertEqual(rc, 0, out + err)
        self.assertEqual(self.host_state()["installed"], pending)
        self.assert_no_token(pending, out, err)

    # -- refusals, each before a write ---------------------------------------

    def refused(self, code, rc, out, err):
        self.assertEqual(rc, 1, out + err)
        self.assertTrue(err.startswith(code + ": "), err)
        self.assertFalse(self.host_state().get("installed"))

    def test_only_a_declared_founder_mints(self):
        for value in (None, False, "true"):
            with self.subTest(goatnet_founder=value):
                self.gvars = {k: v for k, v in self.gvars.items() if k != "goatnet_founder"}
                if value is not None:
                    self.gvars["goatnet_founder"] = value
                self.refused("GB-435", *self.run_driver())
                self.assertEqual(self.argv_log(), "")

    def test_a_posture_with_no_store_copy_is_refused(self):
        for store in ("openbao", "vaultwarden", "custom", "env"):
            with self.subTest(store=store):
                self.refused("GB-436", *self.run_driver(store=store))
                self.assertEqual(self.argv_log(), "")

    def test_a_dead_stored_token_that_is_not_pending_is_never_replaced(self):
        self.envfile.write_text(f"{ENV}=nbp_{'d' * 36}\n")
        self.refused("GB-437", *self.run_driver())
        self.assertEqual(self.host_state().get("compose_calls", 0), 0, "decided before bootstrap-user runs")
        self.assertEqual(self.stored(), "nbp_" + "d" * 36)

    def test_a_held_token_with_no_stored_copy_is_refused(self):
        self.host_state({"rows": 1})
        self.refused("GB-437", *self.run_driver())
        self.assertEqual(self.stored(), "")

    def test_an_unfinished_rotation_is_refused_with_its_command(self):
        self.host_state({"rows": 2})
        rc, out, err = self.run_driver()
        self.refused("GB-437", rc, out, err)
        self.assertIn("bootstrap-token-revoke", err)

    def test_another_management_image_is_refused(self):
        self.host_state({"repo_digest": "netbirdio/management@sha256:" + "0" * 64})
        self.refused("GB-438", *self.run_driver())
        self.assertEqual(self.host_state().get("compose_calls", 0), 0)

    def test_a_store_that_does_not_read_back_switches_nothing_on(self):
        real = drv.read_back
        calls = {"n": 0}
        def lying(*a, **k):
            calls["n"] += 1
            return "" if calls["n"] == 1 else "nbp_" + "x" * 36
        with mock.patch.object(drv, "read_back", side_effect=lying):
            rc, out, err = self.run_driver()
        self.refused("GB-439", rc, out, err)
        self.assertTrue(self.host_state().get("pending"), "the pending token stays for a re-run")
        self.assertIsNotNone(real)

    def test_a_store_that_keeps_its_value_is_refused(self):
        ids = drv.load_identities()
        p = ids.posture()
        with mock.patch.object(p, "set_values", return_value={ENV: "kept"}):
            with mock.patch.object(ids, "posture", return_value=p), mock.patch.object(drv, "load_identities", return_value=ids):
                rc, out, err = self.run_driver()
        self.refused("GB-439", rc, out, err)

    # -- 1password: see DriverMore and OnePasswordUnreadable, through a stand-in op --


class Doctor(unittest.TestCase):
    def test_a_previous_token_outside_a_rotation_is_flagged(self):
        spec2 = importlib.util.spec_from_file_location("goat_posture_t", ROOT / "ops/secrets/posture.py")
        p = importlib.util.module_from_spec(spec2)
        spec2.loader.exec_module(p)
        with tempfile.TemporaryDirectory() as d:
            f = Path(d) / "s.env"
            f.write_text("ZAGROS_NB_API_TOKEN=a\n")
            os.chmod(f, 0o600)
            env = {"GOAT_SECRET_STORE": "env-file", "GOAT_SECRET_ENV_FILE": str(f), "PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", d)}
            with mock.patch.dict(os.environ, env, clear=True):
                self.assertNotIn("secrets:rotation-leftover", {r[1] for r in p.doctor()})
                f.write_text("ZAGROS_NB_API_TOKEN=a\nZAGROS_NB_API_TOKEN_PREVIOUS=b\n")
                rows = {r[1]: r for r in p.doctor()}
                self.assertFalse(rows["secrets:rotation-leftover"][0])
                self.assertIn("ZAGROS_NB_API_TOKEN_PREVIOUS", rows["secrets:rotation-leftover"][2])
                self.assertNotIn("b", rows["secrets:rotation-leftover"][2].replace("by", "").split())




class DriverMore(Driver):
    # Runs Driver's tests again too; cheap, and they share the stand-ins.
    """The review's cases: the one-shot's image, the host, the store that cannot
    be read, the check after install, and 1Password through a stand-in op."""

    def test_a_one_shot_that_would_start_another_image_is_refused(self):
        self.host_state({"declared": "netbirdio/management:0.68.3", "declared_id": "sha256:" + "b" * 64})
        self.refused("GB-438", *self.run_driver())
        self.assertEqual(self.host_state().get("compose_calls", 0), 0)

    def test_a_host_outside_the_group_is_refused(self):
        self.args = ["--host", "ops@192.0.2.99"]
        rc, out, err = self.run_driver()
        self.assertEqual(rc, 1)
        self.assertIn("is not a host of zagros_hosts", err)
        self.assertEqual(self.argv_log(), "")

    def test_the_host_is_never_an_ssh_option(self):
        self.args = ["--host", "-oProxyCommand=touch /tmp/x"]
        rc, out, err = self.run_driver()
        self.assertEqual(rc, 1)
        self.assertEqual(self.argv_log(), "")

    def test_a_store_that_cannot_be_read_stops_the_run(self):
        ids = drv.load_identities()
        p = ids.posture()
        with mock.patch.object(p, "values", side_effect=p.PostureError("locked")), \
                mock.patch.object(ids, "posture", return_value=p), mock.patch.object(drv, "load_identities", return_value=ids):
            rc, out, err = self.run_driver()
        self.assertEqual(rc, 1)
        self.assertIn("cannot be read", err)
        self.assertEqual(self.argv_log(), "", "decided before the host is reached")

    def test_a_token_that_does_not_work_after_install_is_an_error(self):
        self.api_refuses = True
        rc, out, err = self.run_driver()
        self.assertEqual(rc, 1)
        self.assertIn("does not authenticate", err)

    def test_a_reboot_that_cleared_the_pending_token_names_the_way_out(self):
        self.envfile.write_text(f"{ENV}=nbp_{'r' * 36}\n")
        rc, out, err = self.run_driver()
        self.refused("GB-437", rc, out, err)
        self.assertIn(f"remove {ENV} from the store", err)

    def test_under_1password_a_stand_in_op_holds_the_item_and_reads_it_back(self):
        (self.bin / "op").write_text(OP)
        (self.bin / "op").chmod(0o755)
        ids = drv.load_identities()
        with mock.patch.object(drv, "load_identities", return_value=ids), \
                mock.patch.object(ids, "onepassword_account", return_value="fixture"):
            rc, out, err = self.run_driver(store="1password")
        self.assertEqual(rc, 0, out + err)
        token = self.host_state()["installed"]
        item = json.loads((self.state / "op-item.json").read_text())
        self.assertIn(token, [f.get("value") for f in item["fields"]])
        refs = (self.checkout / ".envrc.local.d/30-site-services.refs").read_text()
        self.assertIn(f'export {ENV}="op://', refs)
        self.assert_no_token(token, out, err, refs)
        op_log = (self.state / "op-argv.log").read_text()
        self.assertNotIn(token, op_log)


class OnePasswordUnreadable(DriverMore):
    """A store that does not answer is never read as an empty store."""

    def op_ready(self):
        (self.bin / "op").write_text(OP)
        (self.bin / "op").chmod(0o755)
        ids = drv.load_identities()
        return ids, mock.patch.object(drv, "load_identities", return_value=ids), \
            mock.patch.object(ids, "onepassword_account", return_value="fixture")

    def test_a_signed_out_op_stops_the_run_before_anything(self):
        (self.state / "op-signed-out").write_text("")
        ids, a, b = self.op_ready()
        with a, b:
            rc, out, err = self.run_driver(store="1password")
        self.assertEqual(rc, 1, out + err)
        self.assertIn("op whoami failed", err)
        self.assertEqual(self.argv_log(), "", "the host is never reached")

    def test_a_declared_reference_that_does_not_resolve_stops_the_run(self):
        refs = self.checkout / ".envrc.local.d/30-site-services.refs"
        refs.write_text(f'export {ENV}="op://vault/zagros-netbird/api_token"\n')
        (self.state / "op-read-fails").write_text("")
        ids, a, b = self.op_ready()
        with a, b:
            rc, out, err = self.run_driver(store="1password")
        self.assertEqual(rc, 1, out + err)
        self.assertIn("does not resolve", err)
        self.assertNotIn("GB-437", err, "never steered toward a rotation")
        self.assertEqual(self.argv_log(), "", "the host is never reached")


OP = r'''#!/usr/bin/env python3
import json, os, sys
state_dir = os.environ["SU_STATE"]
item = os.path.join(state_dir, "op-item.json")
with open(os.path.join(state_dir, "op-argv.log"), "a") as f:
    f.write(json.dumps(sys.argv) + "\n")
args = sys.argv[1:]
if "whoami" in args:
    sys.exit(1 if os.path.exists(os.path.join(state_dir, "op-signed-out")) else 0)
if "read" in args and os.path.exists(os.path.join(state_dir, "op-read-fails")):
    print("[ERROR] not signed in", file=sys.stderr); sys.exit(1)
if "item" in args and "get" in args:
    if not os.path.exists(item):
        print("not found", file=sys.stderr); sys.exit(1)
    print(open(item).read()); sys.exit(0)
if "item" in args and ("create" in args or "edit" in args):
    doc = json.loads(sys.stdin.read()); doc.setdefault("id", "item-1")
    open(item, "w").write(json.dumps(doc)); sys.exit(0)
if "read" in args:
    ref = args[-1]
    field = ref.rsplit("/", 1)[-1]
    doc = json.load(open(item))
    v = next((f["value"] for f in doc["fields"] if f.get("label") == field), "")
    sys.stdout.write(v); sys.exit(0 if v else 1)
sys.exit(97)
'''


class DoctorRefs(unittest.TestCase):
    def test_a_previous_reference_line_is_flagged(self):
        spec2 = importlib.util.spec_from_file_location("goat_posture_r", ROOT / "ops/secrets/posture.py")
        p = importlib.util.module_from_spec(spec2)
        spec2.loader.exec_module(p)
        with tempfile.TemporaryDirectory() as d:
            envrc = Path(d) / ".envrc.local.d"
            envrc.mkdir()
            (envrc / "30-site-services.refs").write_text('export ZAGROS_NB_API_TOKEN="op://v/zagros-netbird/api_token"\n')
            env = {"GOAT_SECRET_STORE": "1password", "PATH": os.environ["PATH"], "HOME": d}
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(p, "_main_checkout", return_value=d), \
                    mock.patch.object(p, "_op_rows", return_value=[]):
                self.assertNotIn("secrets:rotation-leftover", {r[1] for r in p.doctor()})
                with open(envrc / "30-site-services.refs", "a") as f:
                    f.write('export ZAGROS_NB_API_TOKEN_PREVIOUS="op://v/zagros-netbird/api_token_previous"\n')
                rows = {r[1]: r for r in p.doctor()}
                self.assertFalse(rows["secrets:rotation-leftover"][0])


if __name__ == "__main__":
    unittest.main()


class OldImage(unittest.TestCase):
    """An image built before the commands fails safe: GB-438 with the person's
    phase 11 steps and the resume command, and nothing run on the host."""

    def test_an_image_without_the_commands_refuses_with_the_manual_steps(self):
        d = Driver("test_a_founder_mints_stores_reads_back_and_switches_on")
        d.setUp()
        try:
            d.host_state({"old_image": True})
            for profile, marker in (("lan", "on a LAN site the dashboard is the dashboard.* name"),
                                    ("internet", "enroll YOUR laptop (netbird up)")):
                d.args = ["--profile", profile, "--resume", "goatbuilder standup dev --site zagros --from 12"]
                rc, out, err = d.run_driver()
                self.assertEqual(rc, 1, out + err)
                self.assertTrue(err.startswith("GB-438: "), err)
                self.assertIn("has no service-user bootstrap commands", err)
                self.assertIn(marker, err)
                self.assertIn("create service user svc-zagros-netbird-infra", err)
                self.assertIn("THEN resume: goatbuilder standup dev --site zagros --from 12", err)
                self.assertEqual(d.host_state().get("compose_calls", 0), 0, "bootstrap-user never ran")
                self.assertEqual(d.stored(), "")
        finally:
            d.tearDown()

    def test_the_manual_steps_are_the_ones_goat_standup_prints(self):
        text = (ROOT / "ops/scripts/goat-standup.sh").read_text()
        rows = re.findall(r'step 11 MANUAL "\[(lan|internet)\][^"]*" \\\n  "([^"]*)"', text)
        self.assertEqual({p for p, _ in rows}, {"lan", "internet"})
        for profile, row in rows:
            a = mock.Mock(site="zagros", group=None, profile=profile, resume="ops/scripts/goat-standup.sh zagros --from 12", owner="$OWNER_EMAIL_STANDUP")
            want = (row.replace("$GROUP", "zagros_hosts").replace("${SITE_UPPER}", "ZAGROS").replace("${SITE}", "zagros")
                    .replace("$SITE", "zagros"))
            self.assertEqual(drv.manual_steps(a), want, profile)
