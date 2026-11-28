#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""What a small computer's end-state check reads, on the machine itself.

docs/design/goatbuilder-build-targets.md sections 3.1 and 3.5. `goatbuilder
doctor --target <type> --site <site> --live` runs this as root over Ansible on
each machine that declares the small-computer family, and `goatbuilder image
<model>` runs it over SSH after each of the builder's steps. The judging is in
crates/goatbuilder/src/small_computer.rs.

It reads and changes nothing, sends nothing, and judges nothing: it prints one
JSON object of facts. A fact that cannot be read is {"error": "<why>"}, never a
guess. Root is needed for the LUKS header and the initramfs; without it those
facts are errors. Standard library only: the machine needs Python 3 and the
tools named in the errors.

usage: small-computer-endstate.py

Two variables are for tests, and are ignored when the script runs as root:
SMALL_COMPUTER_ROOT is read in place of / for files (not for commands), and
SMALL_COMPUTER_PATH replaces the fixed PATH the commands are found on. The
commands get no other variable of the caller's.
"""
import glob
import json
import os
import re
import subprocess
import sys

TESTING = os.geteuid() != 0
SYSTEM_PATH = "/usr/sbin:/usr/bin:/sbin:/bin"
ENV = {"LC_ALL": "C", "PATH": os.environ.get("SMALL_COMPUTER_PATH", SYSTEM_PATH) if TESTING else SYSTEM_PATH}
ROOT = os.environ.get("SMALL_COMPUTER_ROOT", "/") if TESTING else "/"


def at(path):
    """A machine path under ROOT."""
    return os.path.join(ROOT, path.lstrip("/"))


def run(*argv, timeout=60):
    """A command's stdout, or {"error": ...}."""
    try:
        r = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", errors="replace",
                           timeout=timeout, env=ENV, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.TimeoutExpired) as e:
        return {"error": "%s: %s" % (argv[0], e)}
    if r.returncode != 0:
        said = (r.stderr.strip() or r.stdout.strip()).splitlines()
        return {"error": "%s: %s" % (argv[0], said[0] if said else "exit %d" % r.returncode)}
    return r.stdout


def read(path):
    """A file's text, or {"error": ...}."""
    try:
        with open(at(path), encoding="utf-8", errors="replace") as fh:
            return fh.read()
    except OSError as e:
        return {"error": "%s: %s" % (path, e.strerror or e)}


def failed(v):
    return isinstance(v, dict) and "error" in v


def os_release(text):
    """ID and VERSION_ID of an os-release file's text."""
    if failed(text):
        return text
    out = {}
    for line in text.splitlines():
        k, sep, v = line.partition("=")
        if sep and k in ("ID", "VERSION_ID", "PRETTY_NAME"):
            out[k.lower()] = v.strip().strip('"')
    return out


def root_chain():
    """The root filesystem's device, and every device under it to the disk."""
    source = run("findmnt", "-nvo", "SOURCE", "/")
    if failed(source):
        return source
    source = source.strip()
    if not source.startswith("/dev/"):
        return {"error": "the root is %s, not a block device" % source}
    out = run("lsblk", "-J", "-s", "-p", "-o", "NAME,TYPE,FSTYPE", source)
    if failed(out):
        return out
    try:
        tree = json.loads(out)["blockdevices"]
    except (ValueError, KeyError, TypeError) as e:
        return {"error": "lsblk: not its JSON: %s" % e}
    chain = []
    node = tree[0] if tree else None
    while node:
        chain.append({"name": node.get("name"), "type": node.get("type"), "fstype": node.get("fstype")})
        kids = node.get("children") or []
        node = kids[0] if kids else None
    return {"source": source, "chain": chain}


def luks_partition(chain):
    """The partition the root's crypt device sits on, from the chain."""
    if failed(chain):
        return None
    seen_crypt = False
    for d in chain["chain"]:
        if d["type"] == "crypt":
            seen_crypt = True
        elif seen_crypt and d.get("fstype") == "crypto_LUKS":
            return d["name"]
    return None


def luks(device):
    """The LUKS2 header's metadata, as cryptsetup prints it, and its version."""
    if device is None:
        return {"error": "the root is on no LUKS device"}
    text = run("cryptsetup", "luksDump", device)
    if failed(text):
        return text
    m = re.search(r"^Version:\s*(\d+)", text, re.M)
    version = int(m.group(1)) if m else None
    u = re.search(r"^UUID:\s*(\S+)", text, re.M)
    out = {"device": device, "version": version, "uuid": u.group(1) if u else None}
    if version != 2:
        return out
    meta = run("cryptsetup", "luksDump", "--dump-json-metadata", device)
    if failed(meta):
        out["metadata"] = meta
        return out
    try:
        out["metadata"] = header(json.loads(meta))
    except (ValueError, AttributeError, TypeError) as e:
        out["metadata"] = {"error": "cryptsetup: not its JSON: %s" % e}
    return out


def header(meta):
    """The header's shape and primitives: salts, digests and tokens' contents
    are not printed. A token is named by its type alone (a Clevis token holds
    its server's address)."""
    def strip(v):
        if isinstance(v, dict):
            return {k: strip(x) for k, x in v.items() if k not in ("salt", "digest")}
        return v
    return {
        "keyslots": strip(meta.get("keyslots", {})),
        "segments": strip(meta.get("segments", {})),
        "digests": {k: {"type": d.get("type"), "hash": d.get("hash")} for k, d in meta.get("digests", {}).items()},
        "tokens": {k: {"type": t.get("type")} for k, t in meta.get("tokens", {}).items()},
        "requirements": meta.get("config", {}).get("requirements", {}),
    }


def mapping(chain):
    """The active crypt mapping of the root: its cipher and key size."""
    if failed(chain):
        return chain
    names = [d["name"] for d in chain["chain"] if d["type"] == "crypt"]
    if not names:
        return {"error": "the root is on no crypt device"}
    text = run("cryptsetup", "status", names[0])
    if failed(text):
        return text
    out = {"name": names[0]}
    for line in text.splitlines():
        k, sep, v = line.strip().partition(":")
        if sep and k in ("type", "cipher", "keysize"):
            out[k] = v.strip()
    return out


def crypttab(text):
    """/etc/crypttab's entries: name, device, key, options."""
    if failed(text):
        return text
    rows = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        f = line.split()
        rows.append({"name": f[0], "device": f[1] if len(f) > 1 else "", "key": f[2] if len(f) > 2 else "none",
                     "options": f[3].split(",") if len(f) > 3 else []})
    return rows


def initramfs():
    """What the initramfs the board boots holds, of what unlocking needs."""
    for path in ("/boot/firmware/initrd.img", "/boot/initrd.img-" + os.uname().release):
        if os.path.exists(at(path)):
            listing = run("lsinitramfs", at(path), timeout=120)
            if failed(listing):
                return listing
            paths = listing.split()
            names = set(os.path.basename(n) for n in paths)
            return {"file": path, "cryptsetup": "cryptsetup" in names, "ykchalresp": "ykchalresp" in names,
                    "yubikey_unlock": "yubikey_unlock" in names, "dropbear": "dropbear" in names,
                    "clevis": any(n.startswith("clevis") for n in names),
                    # Key files the initramfs carries: cryptsetup-initramfs's own, and the builder's .lek.
                    "keyfiles": sorted(p for p in paths if "cryptroot/keyfiles/" in p or p.endswith(".lek"))}
    return {"error": "no /boot/firmware/initrd.img or /boot/initrd.img-%s" % os.uname().release}


def providers(text):
    """`openssl list -providers`: each provider's name and status."""
    if failed(text):
        return text
    out, cur = {}, None
    for line in text.splitlines():
        if re.match(r"^  \S", line):
            cur = line.strip()
            out[cur] = {}
        elif cur and line.startswith("    ") and ":" in line:
            k, _, v = line.strip().partition(":")
            out[cur][k.strip()] = v.strip()
    return out


SSHD_KEYS = ("ciphers", "macs", "kexalgorithms", "hostkeyalgorithms", "pubkeyacceptedalgorithms",
             "passwordauthentication", "kbdinteractiveauthentication", "permitrootlogin", "pubkeyauthentication",
             "hostbasedauthentication", "gssapiauthentication")


def sshd(text):
    """The settings of `sshd -T` this check reads."""
    if failed(text):
        return text
    out = {}
    for line in text.splitlines():
        k, _, v = line.partition(" ")
        if k in SSHD_KEYS:
            out[k] = v.strip()
    return out


# The command line's words this check reads; nothing else of it is printed.
CMDLINE_KEYS = ("root", "cryptdevice", "fips")


def cmdline(text):
    if failed(text):
        return text
    return [w for w in text.split() if w.split("=", 1)[0] in CMDLINE_KEYS]


def sshd_match():
    """Match blocks in sshd's configuration files: `sshd -T` with no connection
    does not resolve them, so what they change is not read."""
    found = []
    for path in [at("/etc/ssh/sshd_config")] + sorted(glob.glob(at("/etc/ssh/sshd_config.d/*.conf"))):
        try:
            with open(path, encoding="utf-8", errors="replace") as fh:
                for n, line in enumerate(fh, 1):
                    if re.match(r"\s*match\s", line, re.I):
                        found.append("%s:%d" % (path[len(ROOT.rstrip("/")):], n))
        except OSError as e:
            if os.path.exists(path):
                return {"error": "%s: %s" % (path, e.strerror or e)}
    return found


def boot_keyfiles():
    """Key files on the unencrypted boot partition (the builder's USB-key mode reads *.lek there)."""
    return sorted(os.path.basename(p) for p in glob.glob(at("/boot/firmware/*.lek")) + glob.glob(at("/boot/*.lek")))


def md5_by_default():
    """Whether OpenSSL computes MD5 by default: "computes", or "refused" when
    its default properties hold programs to the FIPS provider (it says the
    digest is unsupported). Any other failure is not that answer: an error."""
    out = run("openssl", "dgst", "-md5", "/dev/null")
    if not failed(out):
        return "computes"
    said = out["error"]
    if re.search(r"unsupported|disabled for FIPS|not supported", said, re.I):
        return "refused"
    return {"error": said}


def kernel_fips():
    """/proc/sys/crypto/fips_enabled: 1, 0, or absent (a kernel with no FIPS mode)."""
    if not os.path.exists(at("/proc/sys/crypto/fips_enabled")):
        return "absent"
    v = read("/proc/sys/crypto/fips_enabled")
    return v if failed(v) else v.strip()


def first_boot():
    """Whether cloud-init's first boot finished."""
    status = run("cloud-init", "status", "--long")
    out = {"finished": os.path.exists(at("/var/lib/cloud/instance/boot-finished"))}
    if failed(status):
        out["status"] = status
    else:
        m = re.search(r"^status:\s*(\S+)", status, re.M)
        out["status"] = m.group(1) if m else {"error": "cloud-init status: no status line"}
    return out


def boot_order():
    """The bootloader's BOOT_ORDER (Raspberry Pi EEPROM), when the tool is here."""
    out = run("rpi-eeprom-config")
    if failed(out):
        return out
    m = re.search(r"^BOOT_ORDER=(\S+)", out, re.M)
    return m.group(1) if m else {"error": "rpi-eeprom-config: no BOOT_ORDER"}


def main():
    chain = root_chain()
    device = luks_partition(chain)
    model = read("/proc/device-tree/model")
    print(json.dumps({
        "os": os_release(read("/etc/os-release")),
        "arch": os.uname().machine,
        "board": model if failed(model) else model.strip("\x00 \n"),
        "root": chain,
        "luks": luks(device),
        "mapping": mapping(chain),
        "crypttab": crypttab(read("/etc/crypttab")),
        "initramfs": initramfs(),
        "cmdline": cmdline(read("/proc/cmdline")),
        # What the next boot reads (a step that changes it takes effect then).
        "cmdline_next": cmdline(read("/boot/firmware/cmdline.txt")),
        "kernel_fips": kernel_fips(),
        "openssl_providers": providers(run("openssl", "list", "-providers")),
        "openssl_md5": md5_by_default(),
        "sshd": sshd(run("sshd", "-T")),
        "sshd_match": sshd_match(),
        "boot_keyfiles": boot_keyfiles(),
        "first_boot": first_boot(),
        "boot_order": boot_order(),
        "python": sys.version.split()[0],
        "root_user": os.geteuid() == 0,
    }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
