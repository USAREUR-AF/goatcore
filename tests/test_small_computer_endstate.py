"""ops/scripts/small-computer-endstate.py, the collector `goatbuilder doctor
--target --live` runs on a small computer, run against the stand-in machine of
tests/fixtures/small-computer/machine.sh: real LUKS2 headers made by the real
cryptsetup, stand-in tools for the rest. Needs no root."""
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parent.parent
SCRIPT = REPO / "ops/scripts/small-computer-endstate.py"
MACHINE = REPO / "tests/fixtures/small-computer/machine.sh"

CRYPTSETUP = "/usr/sbin/cryptsetup"
needs_cryptsetup = pytest.mark.skipif(not Path(CRYPTSETUP).exists(), reason="needs cryptsetup at /usr/sbin/cryptsetup")


def machine(tmp_path):
    subprocess.run(["bash", str(MACHINE), str(tmp_path)], check=True, capture_output=True)
    return tmp_path


def facts(d):
    env = dict(os.environ, SMALL_COMPUTER_ROOT=str(d / "root"),
               SMALL_COMPUTER_PATH="%s:/usr/sbin:/usr/bin:/sbin:/bin" % (d / "bin"))
    out = subprocess.run([sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True, check=True)
    return json.loads(out.stdout)


@needs_cryptsetup
def test_a_pi_as_the_builder_leaves_it(tmp_path):
    f = facts(machine(tmp_path))
    assert f["os"] == {"id": "ubuntu", "version_id": "24.04", "pretty_name": "Ubuntu 24.04.4 LTS"}
    assert f["board"] == "Raspberry Pi 5 Model B Rev 1.0"
    assert [d["type"] for d in f["root"]["chain"]] == ["crypt", "part", "disk"]
    luks = f["luks"]
    assert luks["version"] == 2 and luks["device"].endswith("luks.img")
    meta = luks["metadata"]
    assert sorted(meta["keyslots"]) == ["0", "1"]
    for slot in meta["keyslots"].values():
        assert slot["kdf"]["type"] == "pbkdf2" and slot["kdf"]["hash"] == "sha256"
        assert slot["area"]["encryption"] == "aes-xts-plain64" and slot["key_size"] == 64
    assert meta["segments"]["0"]["encryption"] == "aes-xts-plain64"
    assert meta["digests"]["0"] == {"type": "pbkdf2", "hash": "sha256"}
    assert meta["requirements"] == {}
    assert f["mapping"] == {"name": "/dev/mapper/rootfs", "type": "LUKS2", "cipher": "aes-xts-plain64", "keysize": "512 bits"}
    assert f["crypttab"] == [{"name": "rootfs", "device": "/dev/nvme0n1p2", "key": "-", "options": ["luks", "initramfs", "keyscript=yubikey_unlock", "noauto"]}]
    assert f["initramfs"]["ykchalresp"] and f["initramfs"]["yubikey_unlock"] and not f["initramfs"]["dropbear"]
    assert f["cmdline"] == ["root=/dev/mapper/rootfs", "cryptdevice=/dev/nvme0n1p2:rootfs", "fips=1"]
    assert f["kernel_fips"] == "absent"
    assert f["openssl_providers"]["fips"]["status"] == "active"
    assert f["sshd"]["passwordauthentication"] == "no" and "port" not in f["sshd"]
    assert f["first_boot"] == {"finished": True, "status": "done"}
    assert f["boot_order"] == "0xf416"
    # As the builder leaves OpenSSL: no default properties, so MD5 computes.
    assert f["openssl_md5"] == "computes"


@needs_cryptsetup
def test_md5_is_refused_only_on_openssls_own_refusal(tmp_path):
    d = machine(tmp_path)
    tool = d / "bin/openssl"
    text = tool.read_text()
    tool.write_text(text.replace('echo "MD5(/dev/null)= d41d8cd98f00b204e9800998ecf8427e"; exit 0', 'echo "Error setting digest: unsupported" >&2; exit 1'))
    assert facts(d)["openssl_md5"] == "refused"
    tool.write_text(text.replace('echo "MD5(/dev/null)= d41d8cd98f00b204e9800998ecf8427e"; exit 0', 'echo "openssl: cannot load config" >&2; exit 1'))
    assert facts(d)["openssl_md5"] == {"error": "openssl: openssl: cannot load config"}


@needs_cryptsetup
def test_no_salt_digest_or_token_content_is_printed(tmp_path):
    d = machine(tmp_path)
    f = facts(d)

    def keys(v):
        if isinstance(v, dict):
            for k, x in v.items():
                yield k
                yield from keys(x)
        elif isinstance(v, list):
            for x in v:
                yield from keys(x)
    assert not {"salt", "digest"} & set(keys(f["luks"]))
    # A Clevis token holds its server's address: only its type is printed.
    tok = d / "token.json"
    tok.write_text(json.dumps({"type": "clevis", "keyslots": ["1"], "jwe": {"ciphertext": "x"}, "url": "http://192.0.2.5"}))
    subprocess.run([CRYPTSETUP, "token", "import", "--json-file", str(tok), str(d / "luks.img")], check=True, capture_output=True)
    f = facts(d)
    assert f["luks"]["metadata"]["tokens"] == {"0": {"type": "clevis"}}
    # Nothing of the command line but the words judged: no address.
    assert "192.0.2" not in json.dumps(f)


@needs_cryptsetup
def test_an_unfinished_reencryption_and_argon2_are_read_as_they_are(tmp_path):
    d = machine(tmp_path)
    # A reencryption begun and not finished, as a power cut mid-encryption leaves it.
    part = tmp_path / "half.img"
    with open(part, "wb") as fh:
        fh.truncate(40 << 20)
    subprocess.run([CRYPTSETUP, "reencrypt", "--encrypt", "--init-only", "-q", "--type", "luks2", "-c", "aes-xts-plain64", "-s", "512",
                    "--pbkdf", "pbkdf2", "--pbkdf-force-iterations", "1000", "--reduce-device-size", "16M",
                    "--key-file", str(d / "pass"), str(part)], check=True, capture_output=True)
    lsblk = (d / "bin/lsblk").read_text().replace(str(d / "luks.img"), str(part))
    (d / "bin/lsblk").write_text(lsblk)
    meta = facts(d)["luks"]["metadata"]
    assert meta["requirements"] == {"mandatory": ["online-reencrypt-v2"]}
    assert "linear" in [g["type"] for g in meta["segments"].values()]
    d = machine(tmp_path / "argon")
    subprocess.run([CRYPTSETUP, "luksAddKey", "-q", "--pbkdf", "argon2id", "--pbkdf-memory", "32",
                    "--pbkdf-force-iterations", "4", "--key-file", str(d / "pass"), str(d / "luks.img"), str(d / "yk")],
                   check=True, capture_output=True)
    slots = facts(d)["luks"]["metadata"]["keyslots"]
    assert sorted(k["kdf"]["type"] for k in slots.values()) == ["argon2id", "pbkdf2", "pbkdf2"]


@needs_cryptsetup
def test_a_root_that_is_not_encrypted_names_no_luks_device(tmp_path):
    d = machine(tmp_path)
    (d / "bin/lsblk").write_text('#!/bin/sh\necho \'{"blockdevices": [{"name": "/dev/nvme0n1p2", "type": "part", "fstype": "ext4", "children": [{"name": "/dev/nvme0n1", "type": "disk", "fstype": null}]}]}\'\n')
    f = facts(d)
    assert f["luks"] == {"error": "the root is on no LUKS device"}
    assert f["mapping"] == {"error": "the root is on no crypt device"}


@needs_cryptsetup
def test_what_cannot_be_read_is_an_error_never_a_guess(tmp_path):
    d = machine(tmp_path)
    for gone in ["etc/os-release", "etc/crypttab", "proc/cmdline", "boot/firmware/initrd.img"]:
        os.remove(d / "root" / gone)
    for gone in ["openssl", "sshd", "cloud-init"]:
        (d / "bin" / gone).write_text("#!/bin/sh\necho 'cannot' >&2\nexit 1\n")
    f = facts(d)
    for key in ["os", "crypttab", "cmdline", "openssl_providers", "sshd"]:
        assert "error" in f[key], key
    assert "error" in f["first_boot"]["status"]
    assert "error" in f["initramfs"]


@pytest.mark.skipif(subprocess.run(["unshare", "-r", "true"], capture_output=True).returncode != 0, reason="needs an unprivileged user namespace (unshare -r)")
@needs_cryptsetup
def test_as_root_the_test_variables_are_ignored(tmp_path):
    # Root in a user namespace of its own: the collector reads this machine,
    # not the stand-in tree and tools the variables name.
    d = machine(tmp_path)
    env = dict(os.environ, SMALL_COMPUTER_ROOT=str(d / "root"), SMALL_COMPUTER_PATH="%s:/usr/sbin:/usr/bin:/sbin:/bin" % (d / "bin"))
    out = subprocess.run(["unshare", "-r", sys.executable, str(SCRIPT)], env=env, capture_output=True, text=True, check=True, timeout=180)
    f = json.loads(out.stdout)
    assert f["root_user"] is True
    assert f["board"] != "Raspberry Pi 5 Model B Rev 1.0" or Path("/proc/device-tree/model").exists()
    assert f["os"] != facts(d)["os"] or Path("/etc/os-release").read_text().find("24.04.4") >= 0
    assert f.get("boot_order") != "0xf416"


def test_it_runs_on_this_machine_and_prints_json():
    # Unprivileged, on whatever this is: every fact is a value or an error.
    out = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True, check=True, timeout=180)
    f = json.loads(out.stdout)
    assert f["root_user"] is (os.geteuid() == 0)
    assert set(f) >= {"os", "arch", "root", "luks", "mapping", "crypttab", "initramfs", "sshd", "first_boot"}


@pytest.mark.skipif(shutil.which("shellcheck") is None, reason="needs shellcheck")
def test_the_fixture_script_is_shellcheck_clean():
    subprocess.run(["shellcheck", str(MACHINE)], check=True)
