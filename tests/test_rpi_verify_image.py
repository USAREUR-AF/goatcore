"""ops/rpi-base-image/01b-verify-image.sh reads a flashed card back, on a
regular file standing in for the card: a small Pi-shaped image (an MBR, a FAT
boot partition, a root partition of random bytes) made without root by sfdisk
and mtools, "flashed" by copying it and putting 01-flash.sh's copy list on its
boot partition. Each way a card can be wrong is made once and must fail its
own check. ops/ci/test-rpi-flash-linux.sh runs the script on a real block
device after a real flash."""
import os
import re
import shutil
import subprocess
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
BUILDER = REPO / "ops/rpi-base-image"
TOOLS = ["sfdisk", "mformat", "mcopy", "mmd", "mdel", "xz"]

pytestmark = pytest.mark.skipif(any(shutil.which(t) is None for t in TOOLS), reason="needs " + ", ".join(TOOLS))

BOOT = 2048            # the boot partition's first sector
BOOT_SECTORS = 32768   # 16 MiB
ROOT = BOOT + BOOT_SECTORS
KEY = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5TestKeyOnly test"


def copy_list(text):
    """01-flash.sh's BOOT_COPY_LIST and BOOT_OPTIONAL_LIST, as (source, destination)."""
    out = []
    for name in ("BOOT_COPY_LIST", "BOOT_OPTIONAL_LIST"):
        body = re.search(r"^%s='\n(.*?)^'" % name, text, re.S | re.M).group(1)
        out += [tuple(l.split()) for l in body.splitlines() if l.strip()]
    return out


def mcopy(card, src, dst):
    subprocess.run(["mcopy", "-o", "-i", "%s@@%d" % (card, BOOT * 512), str(src), "::/" + dst], check=True,
                   env=dict(os.environ, MTOOLS_SKIP_CHECK="1"))


@pytest.fixture
def world(tmp_path):
    builder = tmp_path / "rpi-base-image"
    shutil.copytree(BUILDER, builder)
    ud = builder / "cloud-init-configs/user-data"
    ud.write_text(ud.read_text().replace("REPLACE_WITH_YOUR_PUBLIC_KEY", KEY))
    raw = tmp_path / "ubuntu.img"
    with open(raw, "wb") as fh:
        fh.truncate(64 << 20)
    subprocess.run(["sfdisk", "-q", str(raw)], input="label: dos\nstart=%d, size=%d, type=c\nstart=%d, type=83\n" % (BOOT, BOOT_SECTORS, ROOT),
                   text=True, check=True)
    subprocess.run(["mformat", "-i", "%s@@%d" % (raw, BOOT * 512), "-T", str(BOOT_SECTORS), "-v", "system-boot", "::"], check=True,
                   env=dict(os.environ, MTOOLS_SKIP_CHECK="1"))
    with open(raw, "r+b") as fh:
        fh.seek(ROOT * 512)
        fh.write(os.urandom(4 << 20))
    # The image's own boot files, as Ubuntu ships some of them.
    for name, text in [("cmdline.txt", "console=tty1 root=LABEL=writable rootwait\n"), ("config.txt", "[all]\narm_64bit=1\n"),
                       ("user-data", "#cloud-config\n"), ("network-config", "version: 2\n")]:
        (tmp_path / name).write_text(text)
        mcopy(raw, tmp_path / name, name)
    image = tmp_path / "ubuntu.img.xz"
    with open(image, "wb") as fh:
        subprocess.run(["xz", "-c", str(raw)], stdout=fh, check=True)
    card = tmp_path / "card"
    shutil.copy(raw, card)
    subprocess.run(["mmd", "-i", "%s@@%d" % (card, BOOT * 512), "::/helper-scripts"], check=True, env=dict(os.environ, MTOOLS_SKIP_CHECK="1"))
    for src, dst in copy_list((builder / "01-flash.sh").read_text()):
        if (builder / src).exists():
            mcopy(card, builder / src, dst)
    for f in sorted((builder / "helper-scripts").iterdir()):
        mcopy(card, f, "helper-scripts/" + f.name)
    return builder, card, image


def verify(builder, card, image):
    r = subprocess.run(["bash", str(builder / "01b-verify-image.sh"), str(card), str(image)], capture_output=True, text=True, timeout=120)
    return r.returncode, re.sub(r"\x1b\[[0-9;]*m", "", r.stdout + r.stderr)


def test_a_card_as_01_flash_leaves_it_holds(world):
    rc, out = verify(*world)
    assert rc == 0, out
    assert "[ok] partition table: the image's" in out
    assert "[ok] root partition: the image's" in out
    assert "[ok] boot partition: 02-clone-to-nvme.sh is this checkout's 02-clone-to-nvme.sh" in out
    assert "[ok] boot partition: helper-scripts/encrypt-rootfs.sh is this checkout's" in out
    assert "put it in the Pi" in out


def test_a_root_partition_byte_changed_fails_the_root_check_alone(world):
    builder, card, image = world
    with open(card, "r+b") as fh:
        fh.seek(ROOT * 512 + 12345)
        b = fh.read(1)
        fh.seek(ROOT * 512 + 12345)
        fh.write(bytes([b[0] ^ 1]))
    rc, out = verify(builder, card, image)
    assert rc == 1, out
    assert [l for l in out.splitlines() if l.startswith("[FAIL]")] == [l for l in out.splitlines() if "[FAIL] root partition:" in l], out


def test_a_partition_table_unlike_the_images_fails(world):
    builder, card, image = world
    subprocess.run(["sfdisk", "-q", "--part-type", str(card), "2", "8e"], check=True, capture_output=True)
    rc, out = verify(builder, card, image)
    assert rc == 1 and "[FAIL] partition table:" in out, out


def test_a_missing_or_changed_boot_file_fails_naming_it(world):
    builder, card, image = world
    env = dict(os.environ, MTOOLS_SKIP_CHECK="1")
    subprocess.run(["mdel", "-i", "%s@@%d" % (card, BOOT * 512), "::/05-enable-fips.sh"], check=True, env=env)
    (builder / "02-clone-to-nvme.sh").write_text("#!/bin/sh\necho changed in the checkout after the flash\n")
    rc, out = verify(builder, card, image)
    assert rc == 1, out
    assert "[FAIL] boot partition: 05-enable-fips.sh is missing" in out
    assert "[FAIL] boot partition: 02-clone-to-nvme.sh is not this checkout's 02-clone-to-nvme.sh" in out


def test_a_changed_or_added_file_of_the_boot_partition_fails(world):
    builder, card, image = world
    tmp = card.parent
    (tmp / "cmdline.txt").write_text("console=tty1 root=LABEL=writable rootwait init=/bin/sh\n")
    mcopy(card, tmp / "cmdline.txt", "cmdline.txt")
    (tmp / "evil.sh").write_text("#!/bin/sh\n")
    mcopy(card, tmp / "evil.sh", "helper-scripts/evil.sh")
    rc, out = verify(builder, card, image)
    assert rc == 1, out
    assert "[FAIL] boot partition: cmdline.txt is not the image's (it was changed after the flash)" in out
    assert "[FAIL] boot partition: helper-scripts/evil.sh is in neither the image nor 01-flash.sh's copy list" in out
    assert "[ok] boot partition: 1 of the image's own files are the image's" in out


def test_a_user_data_with_no_key_fails(world):
    builder, card, image = world
    ud = builder / "cloud-init-configs/user-data"
    ud.write_text(ud.read_text().replace(KEY, "ssh_import_id: []"))
    mcopy(card, ud, "user-data")
    rc, out = verify(builder, card, image)
    assert rc == 1 and "user-data names no SSH public key" in out, out


def test_a_partition_is_not_a_card(world, tmp_path):
    builder, card, image = world
    rc, out = verify(builder, tmp_path, image)
    assert rc == 2 and "neither a disk nor a file" in out, out


def test_a_user_data_with_the_key_placeholder_fails(world):
    builder, card, image = world
    ud = builder / "cloud-init-configs/user-data"
    ud.write_text(ud.read_text().replace(KEY, "REPLACE_WITH_YOUR_PUBLIC_KEY"))
    mcopy(card, ud, "user-data")
    rc, out = verify(builder, card, image)
    assert rc == 1 and "user-data still has the SSH key placeholder" in out, out


def test_what_cannot_be_read_is_exit_2(world, tmp_path):
    builder, card, image = world
    rc, out = verify(builder, tmp_path / "no-such-card", image)
    assert rc == 2 and "does not exist" in out, out
    half = tmp_path / "half.img.xz"
    half.write_bytes(image.read_bytes()[:200])
    rc, out = verify(builder, card, half)
    assert rc == 2 and "not a whole xz file" in out, out


@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="needs shellcheck")
def test_the_script_is_shellcheck_clean():
    subprocess.run(["shellcheck", "-x", "ops/rpi-base-image/01b-verify-image.sh"], cwd=REPO, check=True)
