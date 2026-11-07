"""The secrets session (docs/design/secrets-session.md): start / status / end per store,
and with-secrets using a live session first.

No real store or keychain is touched: `security`, `op`, `bao` and `bw` are stubs
first on PATH; the stub keychain is a directory under a temp dir.

Run: python3 -m unittest tests/test_secrets_session.py -v
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SESSION = os.path.join(REPO, "ops/scripts/secrets-session")
WITH_SECRETS = os.path.join(REPO, "ops/scripts/with-secrets")
POSTURE = os.path.join(REPO, "ops/secrets/posture.py")

OP_TOKEN = "ops_FAKEserviceAccountTokenAAAA1111"
BAO_CHILD = "s.FAKEchildTokenBBBB2222"
BW_KEY = "FAKEbwSessionKeyCCCC3333+/=="

# A stub keychain: `security -i` reads an add line on stdin, find/delete by -s/-a.
SECURITY = r'''#!/usr/bin/env python3
import json, os, shlex, sys
root = os.environ["FAKE_KEYCHAIN"]
with open(os.path.join(root, "argv.log"), "a") as fh:
    fh.write(" ".join(sys.argv[1:]) + "\n")
def opts(words):
    out, i = {}, 0
    while i < len(words):
        w = words[i]
        if w in ("-s", "-a", "-l", "-j", "-w") and i + 1 < len(words):
            out[w] = words[i + 1]; i += 2
        else:
            out[w] = True; i += 1
    return out
def path(o):
    return os.path.join(root, f"{o['-s']}__{o['-a']}.json")
args = sys.argv[1:]
if args == ["-i"]:
    for line in sys.stdin:
        words = shlex.split(line)
        if words and words[0] == "add-generic-password":
            o = opts(words[1:])
            json.dump({"comment": o.get("-j", ""), "secret": o["-w"]}, open(path(o), "w"))
    sys.exit(0)
verb, o = args[0], opts(args[1:])
p = path(o)
if not os.path.exists(p):
    sys.exit(44)
item = json.load(open(p))
if verb == "find-generic-password":
    if "-w" in o:
        print(item["secret"])
    else:
        print(f'    "acct"<blob>="{o["-a"]}"')
        print(f'    "icmt"<blob>="{item["comment"]}"')
        print(f'    "svce"<blob>="{o["-s"]}"')
    sys.exit(0)
if verb == "delete-generic-password":
    os.remove(p); sys.exit(0)
sys.exit(1)
'''

# op: whoami checks the token; run records what op saw, then runs the rest.
OP = r'''#!/usr/bin/env bash
log="$FAKE_KEYCHAIN/op.log"
echo "argv: $*" >>"$log"
case "$1" in
  whoami) [ "${OP_SERVICE_ACCOUNT_TOKEN:-}" = "$FAKE_OP_GOOD" ] ;;
  account) printf '[{"shorthand":"team","url":"example.1password.com"}]' ;;
  run)
    echo "run token=${OP_SERVICE_ACCOUNT_TOKEN:+set} account=${OP_ACCOUNT:-unset}" >>"$log"
    shift; [ "${1:-}" = "--" ] && shift; exec "$@" ;;
esac
'''

BAO = r'''#!/usr/bin/env bash
log="$FAKE_KEYCHAIN/bao.log"
echo "argv: $*" >>"$log"
if [ "$1 $2" = "token create" ]; then
  [ -n "${FAKE_BAO_DENY:-}" ] && { echo "Error creating token: permission denied" >&2; exit 2; }
  printf '{"auth":{"client_token":"%s","lease_duration":%s}}' "$FAKE_BAO_CHILD" "${FAKE_BAO_LEASE:-86400}"
  exit 0
fi
if [ "$1 $2 $3" = "token revoke -self" ]; then
  [ "${BAO_TOKEN:-}" = "$FAKE_BAO_CHILD" ] && echo "revoked-child" >>"$log"
  exit 0
fi
exit 1
'''

BW = r'''#!/usr/bin/env bash
log="$FAKE_KEYCHAIN/bw.log"
echo "argv: $*" >>"$log"
case "$1 ${2:-}" in
  "unlock --check") [ "${BW_SESSION:-}" = "$FAKE_BW_GOOD" ] ;;
  "unlock --raw") printf '%s' "$FAKE_BW_GOOD" ;;
  "lock "*) [ "${BW_SESSION:-}" = "$FAKE_BW_GOOD" ] && echo "locked" >>"$log"; exit 0 ;;
  *) exit 1 ;;
esac
'''

# Stand-ins for the overlay's injectors: record whether their token reached them.
RUNNER = r'''#!/usr/bin/env bash
echo "{name} sees ${VAR:+set}" >>"$FAKE_KEYCHAIN/runners.log"
[ "${{1:-}}" = "--" ] && shift
exec "$@"
'''

# The command: records whether ANY session token reached it.
PROBE = r'''#!/usr/bin/env bash
echo "op=${OP_SERVICE_ACCOUNT_TOKEN:+leaked} bao=${BAO_TOKEN:+leaked} bw=${BW_SESSION:+leaked}" >"$FAKE_KEYCHAIN/probe.out"
'''


def write_exe(path, text):
    with open(path, "w") as fh:
        fh.write(text)
    os.chmod(path, 0o700)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="test-secrets-session-")
        self.kc = os.path.join(self.tmp, "keychain")
        self.bin = os.path.join(self.tmp, "bin")
        os.makedirs(self.kc)
        os.makedirs(self.bin)
        write_exe(os.path.join(self.bin, "security"), SECURITY)
        write_exe(os.path.join(self.bin, "op"), OP)
        write_exe(os.path.join(self.bin, "bao"), BAO)
        write_exe(os.path.join(self.bin, "bw"), BW)
        # A tree shaped like an assembled checkout, so the overlay injectors exist.
        self.tree = os.path.join(self.tmp, "tree")
        os.makedirs(os.path.join(self.tree, "ops/scripts"))
        for sub in ("ops/secrets/openbao", "ops/secrets/vaultwarden"):
            os.makedirs(os.path.join(self.tree, sub))
        shutil.copy2(SESSION, os.path.join(self.tree, "ops/scripts/secrets-session"))
        shutil.copy2(WITH_SECRETS, os.path.join(self.tree, "ops/scripts/with-secrets"))
        shutil.copy2(POSTURE, os.path.join(self.tree, "ops/secrets/posture.py"))
        write_exe(os.path.join(self.tree, "ops/secrets/openbao/run"),
                  RUNNER.replace("{name}", "bao-run").replace("VAR", "BAO_TOKEN").replace("{{", "{").replace("}}", "}"))
        write_exe(os.path.join(self.tree, "ops/secrets/vaultwarden/run"),
                  RUNNER.replace("{name}", "bw-run").replace("VAR", "BW_SESSION").replace("{{", "{").replace("}}", "}"))
        self.probe = os.path.join(self.tmp, "probe")
        write_exe(self.probe, PROBE)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def env(self, **kw):
        e = {
            "PATH": f"{self.bin}:{os.path.dirname(sys.executable)}:/usr/bin:/bin",
            "HOME": self.tmp, "USER": "tester", "FAKE_KEYCHAIN": self.kc,
            "FAKE_OP_GOOD": OP_TOKEN, "FAKE_BAO_CHILD": BAO_CHILD, "FAKE_BW_GOOD": BW_KEY,
            "BAO_ADDR": "https://store.example", "GOAT_BUILDER_CONFIG": os.path.join(self.tmp, "none.yaml"),
        }
        e.update(kw)
        return {k: v for k, v in e.items() if v is not None}

    def session(self, *args, stdin="", **kw):
        return subprocess.run([sys.executable, os.path.join(self.tree, "ops/scripts/secrets-session"), *args],
                              input=stdin, capture_output=True, text=True, env=self.env(**kw))

    def with_secrets(self, **kw):
        return subprocess.run([os.path.join(self.tree, "ops/scripts/with-secrets"), self.probe],
                              capture_output=True, text=True, env=self.env(**kw))

    def read(self, name):
        p = os.path.join(self.kc, name)
        if not os.path.exists(p):
            return ""
        with open(p) as fh:
            return fh.read()

    def entry(self, store):
        p = os.path.join(self.kc, f"goat-secrets-session-{store}__tester.json")
        if not os.path.exists(p):
            return None
        with open(p) as fh:
            return json.load(fh)

    def plant(self, store, token, expires):
        p = os.path.join(self.kc, f"goat-secrets-session-{store}__tester.json")
        with open(p, "w") as fh:
            json.dump({"comment": f"expires={int(expires)}", "secret": token}, fh)

    def assert_no_token_in_argv(self):
        for log in ("argv.log", "op.log", "bao.log", "bw.log"):
            text = self.read(log)
            for tok in (OP_TOKEN, BAO_CHILD, BW_KEY):
                self.assertNotIn(tok, text, f"{tok} appeared in argv ({log})")


class Start(Base):
    def test_1password_token_from_stdin_is_kept_with_its_expiry(self):
        r = self.session("start", "--store", "1password", stdin=OP_TOKEN + "\n")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn(OP_TOKEN, r.stdout + r.stderr)
        e = self.entry("1password")
        self.assertEqual(e["secret"], OP_TOKEN)
        expires = int(e["comment"].split("=")[1])
        self.assertAlmostEqual(expires - time.time(), 24 * 3600, delta=60)
        self.assert_no_token_in_argv()

    def test_hours_are_bounded_to_one_to_twenty_four(self):
        for h in ("0", "25"):
            r = self.session("start", "--store", "1password", "--hours", h, stdin=OP_TOKEN)
            self.assertEqual(r.returncode, 2)
            self.assertIsNone(self.entry("1password"))
        r = self.session("start", "--store", "1password", "--hours", "8", stdin=OP_TOKEN)
        self.assertEqual(r.returncode, 0, r.stderr)
        expires = int(self.entry("1password")["comment"].split("=")[1])
        self.assertAlmostEqual(expires - time.time(), 8 * 3600, delta=60)

    def test_a_token_1password_refuses_is_not_kept(self):
        r = self.session("start", "--store", "1password", stdin="ops_notTheRightTokenZZZZ")
        self.assertEqual(r.returncode, 2)
        self.assertIn("refused", r.stderr)
        self.assertIsNone(self.entry("1password"))

    def test_without_a_keychain_start_refuses_and_writes_nothing(self):
        empty = os.path.join(self.tmp, "nobin")
        os.makedirs(empty)
        r = self.session("start", "--store", "1password", stdin=OP_TOKEN, PATH=empty)
        self.assertEqual(r.returncode, 2)
        self.assertIn("keychain", r.stderr)
        self.assertEqual(os.listdir(self.kc), [])
        self.assertEqual([f for f in os.listdir(self.tmp) if f not in ("keychain", "bin", "tree", "probe", "nobin")], [])

    def test_openbao_mints_a_child_token_and_reports_a_shorter_parent(self):
        r = self.session("start", "--store", "openbao", FAKE_BAO_LEASE="7200")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("-ttl=24h", self.read("bao.log"))
        self.assertIn("ends sooner", r.stdout)
        e = self.entry("openbao")
        self.assertEqual(e["secret"], BAO_CHILD)
        self.assertAlmostEqual(int(e["comment"].split("=")[1]) - time.time(), 7200, delta=60)
        self.assert_no_token_in_argv()

    def test_openbao_refusal_names_the_policy_it_needs(self):
        r = self.session("start", "--store", "openbao", FAKE_BAO_DENY="1")
        self.assertEqual(r.returncode, 2)
        self.assertIn("auth/token/create", r.stderr)
        self.assertIsNone(self.entry("openbao"))

    def test_vaultwarden_keeps_the_unlocked_session(self):
        r = self.session("start", "--store", "vaultwarden", BW_SESSION=BW_KEY)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.entry("vaultwarden")["secret"], BW_KEY)
        self.assert_no_token_in_argv()

    def test_vaultwarden_without_session_or_terminal_refuses(self):
        r = self.session("start", "--store", "vaultwarden")
        self.assertEqual(r.returncode, 2)
        self.assertIsNone(self.entry("vaultwarden"))


class StatusAndEnd(Base):
    def test_status_reads_live_expired_absent(self):
        self.plant("1password", OP_TOKEN, time.time() + 3 * 3600)
        self.plant("openbao", BAO_CHILD, time.time() - 10)
        r = self.session("status")
        self.assertEqual(r.returncode, 0, r.stderr)
        lines = {l.split()[0]: l for l in r.stdout.splitlines()}
        self.assertIn("live", lines["1password"])
        self.assertIn("left", lines["1password"])
        self.assertIn("expired", lines["openbao"])
        self.assertIn("absent", lines["vaultwarden"])
        self.assertNotIn(OP_TOKEN, r.stdout)

    def test_end_revokes_openbao_locks_vaultwarden_and_deletes_every_entry(self):
        for store, tok in (("1password", OP_TOKEN), ("openbao", BAO_CHILD), ("vaultwarden", BW_KEY)):
            self.plant(store, tok, time.time() + 3600)
        r = self.session("end")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("revoked-child", self.read("bao.log"))
        self.assertIn("locked", self.read("bw.log"))
        for store in ("1password", "openbao", "vaultwarden"):
            self.assertIsNone(self.entry(store))
        self.assert_no_token_in_argv()

    def test_an_expired_session_is_ended_when_read(self):
        self.plant("openbao", BAO_CHILD, time.time() - 1)
        r = self.session("token", "openbao")
        self.assertEqual(r.returncode, 1)
        self.assertEqual(r.stdout, "")
        self.assertIsNone(self.entry("openbao"))
        self.assertIn("revoked-child", self.read("bao.log"))


class WithSecrets(Base):
    def test_live_1password_session_reaches_op_run_not_the_command(self):
        self.plant("1password", OP_TOKEN, time.time() + 3600)
        r = self.with_secrets(GOAT_SECRET_STORE="1password", OP_ACCOUNT="example.1password.com",
                              DB_PASS="op://<vault>/item/password")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("run token=set account=unset", self.read("op.log"))
        self.assertIn("op= bao= bw=", self.read("probe.out"))

    def test_no_session_is_todays_path(self):
        r = self.with_secrets(GOAT_SECRET_STORE="1password", OP_ACCOUNT="example.1password.com",
                              DB_PASS="op://<vault>/item/password")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("run token= account=example.1password.com", self.read("op.log"))

    def test_an_expired_session_falls_back_and_is_removed(self):
        self.plant("1password", OP_TOKEN, time.time() - 5)
        r = self.with_secrets(GOAT_SECRET_STORE="1password", OP_ACCOUNT="example.1password.com",
                              DB_PASS="op://<vault>/item/password")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("run token= account=example.1password.com", self.read("op.log"))
        self.assertIsNone(self.entry("1password"))

    def test_sessions_can_be_switched_off_per_command(self):
        self.plant("1password", OP_TOKEN, time.time() + 3600)
        r = self.with_secrets(GOAT_SECRET_STORE="1password", OP_ACCOUNT="example.1password.com",
                              GOAT_SECRETS_SESSION="0", DB_PASS="op://<vault>/item/password")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("run token= account=example.1password.com", self.read("op.log"))

    def test_openbao_and_vaultwarden_sessions_reach_their_injectors_only(self):
        self.plant("openbao", BAO_CHILD, time.time() + 3600)
        self.plant("vaultwarden", BW_KEY, time.time() + 3600)
        r = self.with_secrets(GOAT_SECRET_STORE="openbao",
                              A="bao://kv/x/y#z", B="bw://coll/item/field")
        self.assertEqual(r.returncode, 0, r.stderr)
        runners = self.read("runners.log")
        self.assertIn("bao-run sees set", runners)
        self.assertIn("bw-run sees set", runners)
        self.assertIn("op= bao= bw=", self.read("probe.out"))


if __name__ == "__main__":
    unittest.main()
