"""The goatlite prepared image asks no vendor service on its own.

Ubuntu's cloud image starts two daemons that reach the vendor from the first boot:
systemd-timesyncd asks the vendor's NTP pool (its compiled-in fallback server), and
snapd asks the snap store. Neither is part of a goatnet. A standup from the image sets
the site's time sources itself (the ntp-hygiene role installs chrony with them, and
chrony's package replaces systemd-timesyncd), and nothing a goatlite runs is a snap.

So ops/vm-image/build.sh, after phase 2 and before the image is written:
  - stops and disables systemd-timesyncd,
  - purges snapd, and installs an apt pin so that a later apt run (phase 2 upgrades
    every standup) cannot bring it back,
and its first-boot check fails the build when either is back.

This is a static check over the builder; the first-boot check in build.sh is the
measured one. Run: python3 -m pytest -q tests/test_vm_image_no_vendor_reach.py
"""

import pathlib
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
IMAGE = ROOT / "ops" / "vm-image"
BUILD = IMAGE / "build.sh"
PIN = IMAGE / "files" / "apt-preferences-no-snapd"

# Vendor time services: none may be named by anything the image is made from.
VENDOR_TIME = re.compile(r"ntp\.ubuntu\.com|pool\.ntp\.org|time\.(google|windows|apple|cloudflare)\.com|ntp\.org\b")


def build_lines():
    return BUILD.read_text().splitlines()


def first_index(lines, pattern, start=0):
    rx = re.compile(pattern)
    for i in range(start, len(lines)):
        if rx.search(lines[i]):
            return i
    return -1


class TimesyncdOff(unittest.TestCase):
    def test_build_disables_timesyncd(self):
        lines = build_lines()
        i = first_index(lines, r"systemctl disable --now systemd-timesyncd(\.service)?\b")
        self.assertGreaterEqual(i, 0, "build.sh must stop and disable systemd-timesyncd on the image")

    def test_first_boot_check_fails_when_timesyncd_is_on(self):
        text = BUILD.read_text()
        self.assertRegex(text, r'echo "timesyncd=\$\(systemctl is-enabled systemd-timesyncd',
                         "the first-boot check must read whether systemd-timesyncd is enabled")
        self.assertRegex(text, r'grep -q "\^timesyncd=disabled\$" \|\| die ',
                         "the first-boot check must fail the build when systemd-timesyncd is enabled")


class SnapdGone(unittest.TestCase):
    def test_build_purges_snapd(self):
        i = first_index(build_lines(), r"apt-get .*purge .*\bsnapd\b")
        self.assertGreaterEqual(i, 0, "build.sh must purge snapd from the image")

    def test_pin_keeps_snapd_out(self):
        self.assertTrue(PIN.is_file(), f"{PIN.relative_to(ROOT)} must exist")
        stanza = {}
        for line in PIN.read_text().splitlines():
            if line.strip() and not line.lstrip().startswith("#"):
                k, _, v = line.partition(":")
                stanza[k.strip()] = v.strip()
        self.assertEqual(stanza.get("Package"), "snapd")
        self.assertIn("Pin", stanza)
        self.assertLess(int(stanza.get("Pin-Priority", "0")), 0,
                        "a negative Pin-Priority is what stops apt installing snapd again")

    def test_build_installs_the_pin(self):
        text = BUILD.read_text()
        self.assertIn("files/apt-preferences-no-snapd", text, "build.sh must copy the pin to the build VM")
        self.assertRegex(text, r"install -m 0644 \S*apt-preferences-no-snapd /etc/apt/preferences\.d/\S+",
                         "build.sh must install the pin under /etc/apt/preferences.d")

    def test_first_boot_check_fails_when_snapd_is_present(self):
        text = BUILD.read_text()
        self.assertRegex(text, r'echo "snapd=\$\(dpkg-query ',
                         "the first-boot check must read whether snapd is installed")
        self.assertRegex(text, r'grep -q "\^snapd=absent\$" \|\| die ',
                         "the first-boot check must fail the build when snapd is installed")


class Ordering(unittest.TestCase):
    def test_after_phase_two_and_before_the_image_is_written(self):
        lines = build_lines()
        phase2 = first_index(lines, r"ansible-playbook .*mgmt-host-baseline\.yml")
        written = first_index(lines, r"qemu-img convert -O qcow2 -c ")
        self.assertGreaterEqual(phase2, 0)
        self.assertGreaterEqual(written, 0)
        for what, pat in (("timesyncd off", r"systemctl disable --now systemd-timesyncd"),
                          ("snapd purged", r"apt-get .*purge .*\bsnapd\b"),
                          ("pin installed", r"/etc/apt/preferences\.d/")):
            i = first_index(lines, pat)
            self.assertTrue(phase2 < i < written,
                            f"{what} (line {i + 1}) must come after phase 2 (line {phase2 + 1}) "
                            f"and before the image is written (line {written + 1})")


class NoVendorTimeServer(unittest.TestCase):
    def test_no_vendor_time_server_named(self):
        for p in [BUILD, *sorted((IMAGE / "files").glob("*"))] if (IMAGE / "files").is_dir() else [BUILD]:
            for n, line in enumerate(p.read_text().splitlines(), 1):
                self.assertIsNone(VENDOR_TIME.search(line),
                                  f"{p.relative_to(ROOT)}:{n} names a vendor time server")


if __name__ == "__main__":
    unittest.main()
