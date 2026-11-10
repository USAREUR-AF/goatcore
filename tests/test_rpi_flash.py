#!/usr/bin/env python3
"""Disk checks and boot copy list of ops/rpi-base-image/01-flash.sh, on stubbed host commands.

Nothing here touches a disk: uname, lsblk, findmnt and diskutil are stubs on
PATH, the block-device test is replaced after sourcing, and the copy runs into
a temporary directory. ops/ci/test-rpi-flash-linux.sh is the Linux end-to-end
run against a kernel-backed scratch disk.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[1]
FLASH = ROOT / "ops" / "rpi-base-image" / "01-flash.sh"
PREREQ = ROOT / "ops" / "rpi-base-image" / "00-prerequisites.sh"
BASH = os.environ.get("BASH_UNDER_TEST") or shutil.which("bash")

STUBS = {
    "uname": """
        #!/usr/bin/env bash
        echo "${STUB_OS:-Linux}"
    """,
    "lsblk": """
        #!/usr/bin/env bash
        case "$*" in
          *-nrpso*)     printf '%s\\n' ${STUB_ROOT_CHAIN:-} ;;
          *TYPE*)       echo "${STUB_TYPE:-disk}" ;;
          *RM*)         echo "${STUB_RM:-0}" ;;
          *TRAN*)       echo "${STUB_TRAN:-}" ;;
          *MOUNTPOINT*) printf '%b' "${STUB_MOUNTS:-}" ;;
          *)            exit 64 ;;
        esac
    """,
    "findmnt": """
        #!/usr/bin/env bash
        echo "${STUB_ROOT_SRC:-}"
    """,
    "diskutil": """
        #!/usr/bin/env bash
        [ "$1" = info ] || exit 64
        if [ "$2" = / ]; then printf '%b' "${STUB_DU_ROOT:-}"; exit 0; fi
        [ -n "${STUB_DU_DISK:-}" ] || exit 1
        printf '%b' "$STUB_DU_DISK"
    """,
}

MAC_ROOT = "   Part of Whole:            disk3\\n   APFS Physical Store:      disk0s2\\n"
MAC_SD = ("   Device Location:          Internal\\n   Removable Media:          Removable\\n"
          "   Protocol:                 Secure Digital\\n")
MAC_USB = ("   Device Location:          External\\n   Removable Media:          Fixed\\n"
           "   Protocol:                 USB\\n")
MAC_INTERNAL = ("   Device Location:          Internal\\n   Removable Media:          Fixed\\n"
                "   Protocol:                 Apple Fabric\\n")


class FlashTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        for name, body in STUBS.items():
            p = self.bin / name
            p.write_text(textwrap.dedent(body).lstrip())
            p.chmod(0o755)
        self.image = self.tmp / "ubuntu-test.img.xz"
        self.image.write_bytes(b"not a real image")

    def env(self, **kw):
        e = {k: v for k, v in os.environ.items() if not k.startswith("STUB_")}
        e["PATH"] = f"{self.bin}{os.pathsep}{e['PATH']}"
        e.update(kw)
        return e

    def fn(self, snippet, blocks="", **kw):
        """Source 01-flash.sh, treat the paths in `blocks` as block devices, run `snippet`."""
        script = (
            'source "$1"; SYS_BLOCK="$2"; BLOCKS=" $3 "\n'
            'is_block_device() { case "$BLOCKS" in *" $1 "*) return 0 ;; esac; return 1; }\n'
            + snippet
        )
        return subprocess.run([BASH, "-c", script, "_", str(FLASH), str(self.tmp / "sys"), blocks],
                              capture_output=True, text=True, env=self.env(**kw), timeout=30)

    def cli(self, *args, **kw):
        return subprocess.run([BASH, str(FLASH), *args], capture_output=True, text=True,
                              env=self.env(**kw), timeout=30, stdin=subprocess.DEVNULL)


class DiskPathValidation(FlashTestCase):
    def check(self, os_name, disk):
        return self.fn(f'disk_path_valid {os_name} "{disk}"').returncode == 0

    def test_macos_takes_whole_disks_only(self):
        self.assertTrue(self.check("darwin", "/dev/disk4"))
        self.assertTrue(self.check("darwin", "/dev/disk12"))
        for bad in ("/dev/disk4s1", "/dev/rdisk4", "/dev/sdb", "/dev/mmcblk0", "/tmp/disk4"):
            self.assertFalse(self.check("darwin", bad), bad)

    def test_linux_takes_sd_mmcblk_and_nvme_whole_disks(self):
        for good in ("/dev/sdb", "/dev/sdaa", "/dev/mmcblk0", "/dev/mmcblk1", "/dev/nvme0n1", "/dev/nvme1n2"):
            self.assertTrue(self.check("linux", good), good)
        for bad in ("/dev/sdb1", "/dev/mmcblk0p1", "/dev/nvme0n1p1", "/dev/disk4", "/dev/loop0",
                    "/dev/dm-0", "/tmp/sdb", "sdb"):
            self.assertFalse(self.check("linux", bad), bad)

    def test_boot_partition_suffix(self):
        r = self.fn('for d in /dev/sdb /dev/mmcblk0 /dev/nvme0n1 /dev/disk4; do boot_partition_of "$d"; done')
        self.assertEqual(r.stdout.split(), ["/dev/sdb1", "/dev/mmcblk0p1", "/dev/nvme0n1p1", "/dev/disk4s1"])


class LinuxRefusals(FlashTestCase):
    def refuse(self, disk="/dev/sdb", allow=0, blocks="/dev/sdb", **kw):
        return self.fn(f'linux_disk_refusal "{disk}" {allow}', blocks=blocks, **kw)

    def test_usb_card_reader_unmounted_passes(self):
        r = self.refuse(STUB_RM="1", STUB_TRAN="usb", STUB_MOUNTS="/dev/sdb \\n/dev/sdb1 \\n",
                        STUB_ROOT_SRC="/dev/nvme0n1p2", STUB_ROOT_CHAIN="/dev/nvme0n1p2 /dev/nvme0n1")
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_a_path_that_is_not_a_block_device_is_refused(self):
        r = self.refuse(blocks="", STUB_RM="1", STUB_TRAN="usb")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not a block device", r.stderr)

    def test_the_disk_holding_root_is_refused_even_with_the_override(self):
        for allow in (0, 1):
            r = self.refuse(disk="/dev/sda", blocks="/dev/sda", allow=allow, STUB_RM="1", STUB_TRAN="usb",
                            STUB_ROOT_SRC="/dev/mapper/vg-root",
                            STUB_ROOT_CHAIN="/dev/mapper/vg-root /dev/mapper/crypt /dev/sda3 /dev/sda")
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("holds the running system's root filesystem", r.stderr)

    def test_root_seen_in_the_mount_list_is_refused(self):
        r = self.refuse(STUB_RM="1", STUB_TRAN="usb", STUB_MOUNTS="/dev/sdb \\n/dev/sdb2 /\\n")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("root filesystem", r.stderr)

    def test_a_mounted_partition_is_refused(self):
        r = self.refuse(STUB_RM="1", STUB_TRAN="usb",
                        STUB_MOUNTS="/dev/sdb \\n/dev/sdb1 /media/u/system-boot\\n/dev/sdb2 \\n")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("/dev/sdb1 on /media/u/system-boot", r.stderr)

    def test_a_fixed_sata_disk_is_refused_unless_overridden(self):
        kw = dict(STUB_RM="0", STUB_TRAN="sata", STUB_MOUNTS="/dev/sdb \\n")
        r = self.refuse(**kw)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not removable, USB or an SD card", r.stderr)
        self.assertEqual(self.refuse(allow=1, **kw).returncode, 0)

    def test_a_partition_or_loop_device_is_refused_by_type(self):
        r = self.refuse(STUB_TYPE="loop", STUB_RM="0")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not a whole disk", r.stderr)

    def test_builtin_sd_reader_passes_and_emmc_is_refused(self):
        dev = self.tmp / "sys" / "mmcblk0" / "device"
        dev.mkdir(parents=True)
        (dev / "type").write_text("SD\n")
        kw = dict(disk="/dev/mmcblk0", blocks="/dev/mmcblk0", STUB_RM="0", STUB_TRAN="")
        self.assertEqual(self.refuse(**kw).returncode, 0)
        (dev / "type").write_text("MMC\n")
        r = self.refuse(**kw)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not removable", r.stderr)

    def test_nvme_needs_the_override(self):
        kw = dict(disk="/dev/nvme1n1", blocks="/dev/nvme1n1", STUB_RM="0", STUB_TRAN="nvme")
        self.assertNotEqual(self.refuse(**kw).returncode, 0)
        self.assertEqual(self.refuse(allow=1, **kw).returncode, 0)


class MacRefusals(FlashTestCase):
    def refuse(self, details, allow=0, disk="/dev/disk4"):
        return self.fn(f'darwin_disk_refusal "{disk}" {allow}', STUB_DU_ROOT=MAC_ROOT, STUB_DU_DISK=details)

    def test_sd_and_usb_pass(self):
        self.assertEqual(self.refuse(MAC_SD).returncode, 0)
        self.assertEqual(self.refuse(MAC_USB).returncode, 0)

    def test_internal_fixed_disk_is_refused(self):
        r = self.refuse(MAC_INTERNAL)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("not removable", r.stderr)

    def test_root_physical_store_is_refused_even_with_the_override(self):
        r = self.refuse(MAC_USB, allow=1, disk="/dev/disk0")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("root filesystem", r.stderr)

    def test_unknown_disk_is_refused(self):
        r = self.refuse("")
        self.assertNotEqual(r.returncode, 0)


class CommandLine(FlashTestCase):
    def test_a_regular_file_is_refused_before_anything_else(self):
        target = self.tmp / "disk.img"
        target.write_bytes(b"\0" * 1024)
        for os_name in ("Linux", "Darwin"):
            r = self.cli("--dry-run", str(target), str(self.image), STUB_OS=os_name)
            self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
            self.assertIn("Invalid disk path", r.stderr)
        self.assertEqual(target.read_bytes(), b"\0" * 1024)

    def test_a_linux_disk_name_that_does_not_exist_is_refused(self):
        r = self.cli("--dry-run", "/dev/sdzz", str(self.image), STUB_OS="Linux")
        self.assertEqual(r.returncode, 1)
        self.assertIn("not a block device", r.stderr)

    def test_a_partition_is_refused(self):
        for os_name, disk in (("Linux", "/dev/sdb1"), ("Linux", "/dev/mmcblk0p1"), ("Darwin", "/dev/disk4s1")):
            r = self.cli("--dry-run", disk, str(self.image), STUB_OS=os_name)
            self.assertEqual(r.returncode, 1)
            self.assertIn("Invalid disk path", r.stderr)

    def test_macos_dry_run_on_an_sd_card_prints_the_plan(self):
        r = self.cli("--dry-run", "/dev/disk4", str(self.image), STUB_OS="Darwin",
                     STUB_DU_ROOT=MAC_ROOT, STUB_DU_DISK=MAC_SD)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("/dev/rdisk4", r.stdout)
        self.assertIn("Dry run: nothing written", r.stdout)

    def test_unsupported_os_is_refused(self):
        r = self.cli("--dry-run", "/dev/sdb", str(self.image), STUB_OS="FreeBSD")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("Unsupported OS", r.stderr)


class BootCopyList(FlashTestCase):
    def test_copy_lands_every_step_script_and_helper_scripts(self):
        boot = self.tmp / "boot"
        boot.mkdir()
        r = self.fn(f'copy_boot_files "{boot}"')
        self.assertEqual(r.returncode, 0, r.stderr)
        names = sorted(str(p.relative_to(boot)) for p in boot.rglob("*") if p.is_file())
        expected = [
            "02-clone-to-nvme.sh", "03-prepare-encryption.sh", "03b-headless-dropbear-unlock.sh",
            "04-enroll-yubikey.sh", "05-enable-fips.sh", "encrypt-rootfs-unlock.sh", "encrypt-rootfs.sh",
            "helper-scripts/encrypt-rootfs-unlock.sh", "helper-scripts/encrypt-rootfs.sh",
            "network-config", "user-data",
        ]
        self.assertEqual(names, expected)

    def test_every_step_script_in_the_directory_is_on_the_list(self):
        on_disk = sorted(p.name for p in FLASH.parent.glob("0[2-9]*.sh"))
        r = self.fn('printf "%s\\n" "$BOOT_COPY_LIST"')
        listed = sorted({line.split()[1] for line in r.stdout.splitlines() if line.strip()
                         and line.split()[1][:2].isdigit()})
        self.assertEqual(listed, on_disk)
        self.assertNotIn("04-post-encryption-setup.sh", r.stdout)


class Prerequisites(unittest.TestCase):
    def test_no_bsd_only_sed_and_no_darwin_only_exit(self):
        text = PREREQ.read_text()
        self.assertNotIn("sed -i ''", text)
        self.assertNotIn("is for macOS only", text)

    def test_linux_package_names_and_commands(self):
        script = textwrap.dedent('''
            eval "$(sed -n '/^linux_pkg_name()/,/^}/p;/^linux_install_cmd()/,/^}/p;/^sed_inplace()/,/^}/p' "$1")"
            linux_pkg_name apt xz; linux_pkg_name dnf xz; linux_pkg_name pacman pv
            linux_install_cmd apt xz-utils pv; linux_install_cmd dnf nmap; linux_install_cmd pacman curl
            f="$2"; printf 'key: REPLACE_WITH_YOUR_PUBLIC_KEY\\n' > "$f"
            sed_inplace "s|REPLACE_WITH_YOUR_PUBLIC_KEY|ssh-ed25519 TEST-ONLY-NOT-A-KEY|g" "$f"
            cat "$f"; ls "$f.bak" 2>/dev/null || echo no-backup
        ''')
        with tempfile.TemporaryDirectory() as d:
            r = subprocess.run([BASH, "-c", script, "_", str(PREREQ), str(Path(d) / "user-data")],
                               capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.splitlines(), [
            "xz-utils", "xz", "pv",
            "sudo apt-get install -y xz-utils pv", "sudo dnf install -y nmap", "sudo pacman -S --needed curl",
            "key: ssh-ed25519 TEST-ONLY-NOT-A-KEY", "no-backup",
        ])


if __name__ == "__main__":
    unittest.main()
