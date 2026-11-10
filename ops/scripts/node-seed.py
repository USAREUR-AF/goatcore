#!/usr/bin/env python3
"""node-seed.py — an Ubuntu autoinstall seed for one host of a node class
(docs/design/goatbuilder-speed.md, sink 5; classes in
ops/goatbuilder/node-classes.yaml).

Status: WIP, not for production use — gate: a timed host baseline on a seeded
Karbon 801 against a stock one, recorded as a measurement.

usage: node-seed.py <site> --host <inventory host> --class <class>
                    [--out DIR] [--no-cache] [--disk-serial SERIAL] [--print]

Writes a NoCloud seed (user-data + meta-data, mode 0600) to
<state dir>/seeds/<host>/ unless --out is given (the state dir is
deploy.state_dir in ops/goatbuilder/goatbuilder.yaml, else ~/.goat/state). Put both files on a
volume labelled CIDATA (e.g. `cloud-localds seed.img user-data meta-data`)
beside the Ubuntu Server installer and boot; the installer runs unattended.

What the seed carries:
  - storage: LUKS-encrypted LVM on the class's disk (crypto_LUKS -> ubuntu-vg),
    the layout host-site-node-disk-encryption.md §2 asserts. The passphrase is
    <HOST>_LUKS_PASS from the environment (the secrets posture supplies it, as
    for the luks plays); it is refused when unset. The seed file therefore holds
    it: it is written 0600 and belongs on the install media only.
  - apt: the site cache (http://<site_cache_host>:3142) when the site's
    group_vars declare site_cache_host and --no-cache is not given.
  - packages: the class's packages plus the role default lists it names.
  - the account the project reaches a new machine as (deploy.ssh_user in
    ops/goatbuilder/goatbuilder.yaml, else `goat`), key-only SSH from the platform-admin roster
    (PLATFORM_ADMIN_SSH_PUBKEYS), passwordless sudo, locked password: the
    day-1 access the baseline play expects. An empty roster is refused.
  - /etc/goat/node-seed.json: class, site, host and the seed's hash, so a
    measurement can tell a seeded host from a stock one.
Not in the seed: Ubuntu Pro, FIPS, the TPM2/Tang unlock slot. The existing
plays do those, unchanged.

exit: 0 written · 1 refused (named) · 2 usage
"""
import argparse
import hashlib
import json
import os
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("node-seed: needs PyYAML (pip3 install pyyaml)")

REPO = Path(__file__).resolve().parents[2]


def env_name(host):
    return re.sub(r"[^A-Za-z0-9]", "_", host).upper() + "_LUKS_PASS"


def role_list(repo, spec):
    role, var = spec.split(":", 1)
    d = yaml.safe_load((repo / "ops/ansible/roles" / role / "defaults/main.yml").read_text()) or {}
    v = d.get(var)
    if not isinstance(v, list):
        raise ValueError(f"{spec}: not a list in roles/{role}/defaults/main.yml")
    return [str(x) for x in v]


def class_packages(repo, cls):
    out = []
    for p in list(cls.get("packages", [])) + [x for s in cls.get("package_lists", []) for x in role_list(repo, s)]:
        if p not in out:
            out.append(p)
    return out


def site_cache_host(repo, site):
    gv = repo / "ops/ansible/group_vars" / (site.replace("-", "_") + "_hosts") / "vars.yml"
    if not gv.exists():
        return ""
    d = yaml.safe_load(gv.read_text()) or {}
    v = d.get("site_cache_host", "")
    return v if isinstance(v, str) and "{{" not in v else ""


def declared(repo, key):
    """deploy.<key> in the checkout's ops/goatbuilder/goatbuilder.yaml, as the
    builder reads it; '' when the file or the key is absent."""
    try:
        cfg = yaml.safe_load((Path(repo) / "ops/goatbuilder/goatbuilder.yaml").read_text()) or {}
    except (OSError, yaml.YAMLError):
        return ""
    return str((cfg.get("deploy") or {}).get(key) or "").strip()


def seed_user(repo):
    """The account the seed creates: the one the project reaches a new machine
    as (deploy.ssh_user), else `goat`."""
    return declared(repo, "ssh_user") or "goat"


def state_dir(repo):
    """Where this machine keeps the project's local state: deploy.state_dir, else ~/.goat/state."""
    return Path(os.path.expanduser(declared(repo, "state_dir") or "~/.goat/state"))


def build(repo, site, host, cls_name, passphrase, keys, cache, disk_serial="", user=""):
    """The autoinstall user-data as a dict, and the meta-data text."""
    classes = (yaml.safe_load((repo / "ops/goatbuilder/node-classes.yaml").read_text()) or {}).get("classes") or {}
    if cls_name not in classes:
        raise ValueError(f"unknown class {cls_name!r}; declared: {', '.join(sorted(classes))}")
    cls = classes[cls_name]
    match = {"serial": disk_serial} if disk_serial else dict(cls.get("disk_match") or {"size": "largest"})
    packages = class_packages(repo, cls)
    marker = {"class": cls_name, "site": site, "host": host, "unlock_class": cls.get("unlock_class", ""),
              "packages": packages}
    ai = {
        "version": 1,
        "locale": "en_US.UTF-8",
        "keyboard": {"layout": "us"},
        "refresh-installer": {"update": False},
        "source": {"id": "ubuntu-server"},
        "storage": {"layout": {"name": "lvm", "match": match, "password": passphrase}},
        "ssh": {"install-server": True, "allow-pw": False},
        "packages": packages,
        "user-data": {
            "hostname": host,
            "users": [{
                "name": user or seed_user(repo),
                "groups": ["sudo"],
                "shell": "/bin/bash",
                "lock_passwd": True,
                "sudo": "ALL=(ALL) NOPASSWD:ALL",
                "ssh_authorized_keys": keys,
            }],
        },
    }
    if cache:
        ai["apt"] = {"proxy": f"http://{cache}:3142"}
    # The hash covers everything but the passphrase; a late command writes it
    # onto the host, so the host names the seed that built it.
    marker["seed_sha256"] = hashlib.sha256(json.dumps({k: v for k, v in ai.items() if k != "storage"}, sort_keys=True).encode()).hexdigest()
    ai["late-commands"] = [
        "mkdir -p /target/etc/goat",
        "printf '%s\\n' " + _shq(json.dumps(marker, sort_keys=True)) + " > /target/etc/goat/node-seed.json",
    ]
    meta = f"instance-id: goat-seed-{host}\nlocal-hostname: {host}\n"
    return {"autoinstall": ai}, meta, marker


def _shq(s):
    return "'" + s.replace("'", "'\"'\"'") + "'"


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("site")
    ap.add_argument("--host", required=True)
    ap.add_argument("--class", dest="cls", required=True)
    ap.add_argument("--out")
    ap.add_argument("--no-cache", action="store_true")
    ap.add_argument("--disk-serial", default="")
    ap.add_argument("--repo", default=str(REPO))
    ap.add_argument("--print", action="store_true", help="print the seed with the passphrase masked; write nothing")
    a = ap.parse_args(argv)
    repo = Path(a.repo)

    var = env_name(a.host)
    passphrase = os.environ.get(var, "")
    if not passphrase and not a.print:
        print(f"node-seed: {var} is not set. The host's LUKS passphrase comes from the secrets store "
              f"(run under ops/scripts/with-secrets, or export it); refusing to write a seed without it.", file=sys.stderr)
        return 1
    keys = [k.strip() for k in os.environ.get("PLATFORM_ADMIN_SSH_PUBKEYS", "").split("\n") if k.strip()]
    if not keys:
        print("node-seed: PLATFORM_ADMIN_SSH_PUBKEYS is empty; a seeded host would admit nobody. "
              "Load .envrc.local.d/45-platform-admin-keys.envrc and re-run.", file=sys.stderr)
        return 1
    cache = "" if a.no_cache else site_cache_host(repo, a.site)
    try:
        user_data, meta, marker = build(repo, a.site, a.host, a.cls, passphrase or "<masked>", keys, cache, a.disk_serial)
    except (ValueError, OSError) as e:
        print(f"node-seed: {e}", file=sys.stderr)
        return 1
    text = "#cloud-config\n" + yaml.safe_dump(user_data, sort_keys=False, width=100000)
    if a.print:
        print(text.replace(passphrase, "<masked>") if passphrase else text)
        return 0
    out = Path(a.out) if a.out else state_dir(repo) / "seeds" / a.host
    out.mkdir(parents=True, exist_ok=True)
    for name, body in (("user-data", text), ("meta-data", meta)):
        p = out / name
        fd = os.open(p, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(body)
        os.chmod(p, 0o600)
    print(f"node-seed: {a.host} ({a.cls}) -> {out}/user-data + meta-data (0600; holds the LUKS passphrase)")
    print(f"  packages: {len(marker['packages'])} · apt cache: {('http://' + cache + ':3142') if cache else 'none (upstream mirrors)'} · seed {marker['seed_sha256'][:12]}")
    print(f"  media: cloud-localds {out}/seed.img {out}/user-data {out}/meta-data  (a volume labelled CIDATA)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
