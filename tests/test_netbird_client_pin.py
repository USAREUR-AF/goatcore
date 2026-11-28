"""netbird-client-pin holds the package only after its reinstall has finished.

The role reinstalls the pinned client in the background (async, poll 0), because
the package's restart of the daemon drops the mesh path the play arrives over.
It then checks that `netbird version` reports the pin and holds the package with
dpkg. The new binary is on disk as soon as dpkg has unpacked it, while apt-get is
still configuring the package and holding dpkg's lock, so the version check could
pass and the hold could run against a busy dpkg: a founder's standup stopped in
phase 2 on `dpkg --set-selections` (rc 32, broken pipe) with the package itself
installed. A reinstall that failed after unpacking was not seen at all.

Each case runs the role's task file on localhost with stand-ins first on PATH
for netbird, dpkg-query, apt-get, apt-cache, apt-mark, systemctl and dpkg. The
stand-in apt-get unpacks the new binary at once and holds a lock file for a few
seconds; the stand-in dpkg refuses --set-selections while that lock is held, as
the real dpkg does. Become is off; nothing outside the test's directory is
touched.

Checked:
  - a reinstall still running when the binary already reports the pin: the
    hold waits for it and lands;
  - a reinstall that fails after unpacking: the play fails, naming the job,
    and the package is not held;
  - a host already at the pin: no reinstall, the hold lands.

Run: python3 -m pytest tests/test_netbird_client_pin.py
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]
TASKS = ROOT / "ops/ansible/roles/netbird-client-pin/tasks/main.yml"
PIN = "0.75.1"

STUBS = {
    "netbird": """\
        [ "$1" = version ] && cat "$NB_STATE/binary"
        """,
    "dpkg-query": """\
        cat "$NB_STATE/dpkg"
        """,
    "apt-cache": """\
        printf 'netbird:\\n  Candidate: %s\\n' "$(cat "$NB_STATE/candidate")"
        """,
    "apt-mark": """\
        exit 0
        """,
    "systemctl": """\
        exit 0
        """,
    # update: nothing. install: the binary is unpacked at once; configuring the
    # package then holds dpkg's lock for NB_APT_SECONDS; NB_APT_RC is the result.
    "apt-get": """\
        case " $* " in *" install "*) ;; *) exit 0 ;; esac
        touch "$NB_STATE/lock"
        cat "$NB_STATE/candidate" > "$NB_STATE/binary"
        sleep "${NB_APT_SECONDS:-0}"
        if [ "${NB_APT_RC:-0}" = 0 ]; then cat "$NB_STATE/candidate" > "$NB_STATE/dpkg"; fi
        rm -f "$NB_STATE/lock"
        exit "${NB_APT_RC:-0}"
        """,
    "dpkg": """\
        case "$1" in
          --get-selections) printf '%s\\t%s\\n' "$2" "$(cat "$NB_STATE/selection")" ;;
          --set-selections)
            if [ -e "$NB_STATE/lock" ]; then
              echo "dpkg: error: dpkg frontend lock was locked by another process" >&2
              exit 2
            fi
            read -r name sel; printf '%s' "$sel" > "$NB_STATE/selection" ;;
          *) exit 0 ;;
        esac
        """,
}


def require_ansible_playbook(case):
    if shutil.which("ansible-playbook"):
        return
    if os.environ.get("GOAT_TEST_REQUIRE_ANSIBLE") == "1":
        case.fail("GOAT_TEST_REQUIRE_ANSIBLE=1 but ansible-playbook is not on PATH")
    case.skipTest("ansible-playbook is required")


class NetbirdClientPin(unittest.TestCase):

    def run_role(self, binary, dpkg, apt_seconds=0, apt_rc=0):
        """Run the role's tasks with the host's client at `binary` (running) and
        `dpkg` (registered). Returns (result, state directory)."""
        require_ansible_playbook(self)
        d = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        state, stubs = d / "state", d / "bin"
        state.mkdir()
        stubs.mkdir()
        for name, body in STUBS.items():
            p = stubs / name
            p.write_text("#!/bin/sh\n" + textwrap.dedent(body))
            p.chmod(0o755)
        (state / "binary").write_text(binary)
        (state / "dpkg").write_text(dpkg)
        (state / "candidate").write_text(PIN)
        (state / "selection").write_text("install")
        play = d / "play.yml"
        play.write_text(textwrap.dedent(f"""\
            - hosts: localhost
              connection: local
              gather_facts: false
              vars:
                netbird_version: "{PIN}"
                netbird_log_rotate_enabled: false
              tasks:
                - ansible.builtin.include_tasks: {TASKS}
            """))
        env = dict(os.environ)
        env.update({
            "PATH": f"{stubs}:{env.get('PATH', '')}",
            "NB_STATE": str(state),
            "NB_APT_SECONDS": str(apt_seconds),
            "NB_APT_RC": str(apt_rc),
            "ANSIBLE_LOCALHOST_WARNING": "false",
            "ANSIBLE_INVENTORY_UNPARSED_WARNING": "false",
            "ANSIBLE_ASYNC_DIR": str(d / "async"),
            "ANSIBLE_LOCAL_TEMP": str(d / "tmp"),
            "ANSIBLE_REMOTE_TEMP": str(d / "tmp"),
        })
        r = subprocess.run(["ansible-playbook", "-i", "localhost,", str(play)],
                           cwd=d, env=env, capture_output=True, text=True, timeout=300)
        return r, state

    def test_the_hold_waits_for_a_reinstall_still_configuring(self):
        # The binary reports the pin the moment it is unpacked; dpkg stays busy
        # for longer than the play's wait for the connection (10 s).
        r, state = self.run_role(binary="0.70.0", dpkg="0.70.0", apt_seconds=20)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:] + r.stderr[-2000:])
        self.assertEqual((state / "selection").read_text(), "hold")
        self.assertEqual((state / "dpkg").read_text(), PIN)

    def test_a_reinstall_that_fails_after_unpacking_fails_the_play(self):
        r, state = self.run_role(binary="0.70.0", dpkg="0.70.0", apt_seconds=12, apt_rc=100)
        self.assertNotEqual(r.returncode, 0, r.stdout[-3000:])
        self.assertIn("Wait for the reinstall to finish", r.stdout)
        self.assertEqual((state / "selection").read_text(), "install")

    def test_a_host_at_the_pin_is_held_without_a_reinstall(self):
        r, state = self.run_role(binary=PIN, dpkg=PIN)
        self.assertEqual(r.returncode, 0, r.stdout[-3000:] + r.stderr[-2000:])
        self.assertNotIn("changed: [localhost]", r.stdout.split("Reinstall pinned")[1].split("TASK")[0])
        self.assertEqual((state / "selection").read_text(), "hold")


if __name__ == "__main__":
    unittest.main()
