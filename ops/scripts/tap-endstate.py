#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""What a network tap's end-state check reads, on the machine itself.

docs/design/goatbuilder-build-targets.md section 3.8. `goatbuilder doctor
--target <type> --site <site> --live` runs this on each tap over Ansible and
judges what it prints (crates/goatbuilder/src/tap_machine.rs). It reads and
changes nothing, sends nothing, and judges nothing: one JSON object of facts.
A fact that cannot be read is {"error": "<why>"}, never a guess. Standard
library only: the machine needs Python 3 and the tools named in the errors.

usage: tap-endstate.py --ports <p1>[,<p2>] --management <interface>
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys


def text(path):
    with open(path, encoding="utf-8", errors="replace") as fh:
        return fh.read().strip()


def number(path):
    try:
        return int(text(path), 0)
    except (OSError, ValueError) as e:
        return {"error": "%s: %s" % (path, e)}


def run(*argv):
    """A command's stdout, or {"error": …}."""
    try:
        r = subprocess.run(argv, capture_output=True, text=True, timeout=20,
                           env=dict(os.environ, LC_ALL="C", PATH="/usr/sbin:/usr/bin:/sbin:/bin"))
    except (OSError, subprocess.TimeoutExpired) as e:
        return {"error": "%s: %s" % (argv[0], e)}
    if r.returncode != 0:
        said = (r.stderr.strip() or r.stdout.strip()).splitlines()
        return {"error": "%s: %s" % (argv[0], said[0] if said else "exit %d" % r.returncode)}
    return r.stdout


def run_json(*argv):
    out = run(*argv)
    if isinstance(out, dict):
        return out
    try:
        return json.loads(out or "[]")
    except ValueError as e:
        return {"error": "%s: not JSON: %s" % (argv[0], e)}


def interface(name):
    sys_dir = "/sys/class/net/" + name
    if not os.path.isdir(sys_dir):
        return {"exists": False}
    addrs = run_json("ip", "-j", "addr", "show", "dev", name)
    out = {
        "exists": True,
        "addresses": addrs if isinstance(addrs, dict) else [
            "%s/%s" % (a.get("local"), a.get("prefixlen")) for i in addrs for a in i.get("addr_info") or []],
        "ipv6_disabled": number("/proc/sys/net/ipv6/conf/%s/disable_ipv6" % name),
        "master": os.path.basename(os.path.realpath(sys_dir + "/master")) if os.path.exists(sys_dir + "/master") else None,
    }
    try:
        out["speed"] = int(text(sys_dir + "/speed"))
    except (OSError, ValueError):
        out["speed"] = None    # no carrier, or a virtual interface
    features = run("ethtool", "-k", name)
    if isinstance(features, dict):
        out["offloads"] = features
    else:
        got = dict(re.findall(r"^\s*([a-z0-9-]+): (on|off)", features, re.M))
        out["offloads"] = {k: got.get(k) for k in ("generic-receive-offload", "large-receive-offload", "rx-gro-hw", "rx-gro-list")}
    eee = run("ethtool", "--show-eee", name)
    if isinstance(eee, dict):
        # A driver with no Energy-Efficient Ethernet has none to turn off.
        none = re.search(r"not supported|Operation not supported|No such device or address", eee["error"])
        out["eee"] = "not supported" if none else eee
    else:
        m = re.search(r"EEE status:\s*([a-z ]+)", eee)
        out["eee"] = m.group(1).strip() if m else {"error": "ethtool --show-eee %s: no status" % name}
    return out


def bridges():
    out = {}
    for d in sorted(glob.glob("/sys/class/net/*/bridge")):
        name = os.path.basename(os.path.dirname(d))
        out[name] = {
            "ports": sorted(os.path.basename(p) for p in glob.glob("/sys/class/net/%s/brif/*" % name)),
            "stp": number(d + "/stp_state"),
            "forward_delay": number(d + "/forward_delay"),
            "group_fwd_mask": number(d + "/group_fwd_mask"),
        }
    return out


def routes():
    out = []
    for fam in ("-4", "-6"):
        got = run_json("ip", "-j", fam, "route", "show", "table", "all")
        if isinstance(got, dict):
            return got
        out += [{"family": fam[1:], "dst": r.get("dst"), "dev": r.get("dev"), "type": r.get("type")} for r in got]
        # A multipath route carries its devices in its next hops.
        out += [{"family": fam[1:], "dst": r.get("dst"), "dev": h.get("dev"), "type": r.get("type")}
                for r in got for h in r.get("nexthops") or []]
    return [r for r in out if r["dev"]]


def egress_devices():
    """{(family, table, chain): devices} of every egress chain, from nft's text.
    nft 1.0.9 (Ubuntu 24.04) leaves a chain's devices out of its JSON."""
    listed = run("nft", "list", "ruleset")
    if isinstance(listed, dict):
        return listed
    out, table, chain = {}, None, None
    for line in listed.splitlines():
        m = re.match(r"\s*table (\S+) (\S+) \{", line)
        if m:
            table = m.groups()
        m = re.match(r"\s*chain (\S+) \{", line)
        if m:
            chain = m.group(1)
        m = re.search(r"hook egress devices? (?:= )?(\{[^}]*\}|\S+)", line)
        if m and table and chain:
            out[table + (chain,)] = re.findall(r"[^\s{},\"]+", m.group(1))
    return out


def egress(name):
    """How the port's egress is dropped, read two ways: an nftables netdev
    egress chain, and a traffic-control egress filter. Either may be used."""
    out = {"nft": None, "tc": None}
    rules = run_json("nft", "-j", "list", "ruleset")
    if isinstance(rules, dict) and "error" in rules:
        out["nft"] = rules
    else:
        out["nft"] = False
        chains = {}
        from_text = None
        for item in rules.get("nftables", []) if isinstance(rules, dict) else []:
            c = item.get("chain")
            if c and c.get("hook") == "egress":
                devs = c.get("dev") if isinstance(c.get("dev"), list) else [c.get("dev")]
                if devs == [None]:
                    from_text = egress_devices() if from_text is None else from_text
                    if "error" in from_text:
                        out["nft"] = from_text
                        break
                    devs = from_text.get((c.get("family"), c.get("table"), c.get("name")), [])
                if name in devs:
                    chains[(c.get("family"), c.get("table"), c.get("name"))] = c.get("policy")
        # A chain drops everything when its policy is drop and no rule accepts,
        # or when its first rule is a bare drop.
        first, accepts = {}, set()
        for item in rules.get("nftables", []) if isinstance(rules, dict) else []:
            r = item.get("rule")
            if not r:
                continue
            key = (r.get("family"), r.get("table"), r.get("chain"))
            # A counter decides nothing: a rule that only counts is not the first verdict.
            expr = [e for e in r.get("expr") or [] if "counter" not in e]
            if key not in first and expr:
                first[key] = expr
            if any(v in e for e in expr for v in ("accept", "return", "jump", "goto", "queue", "fwd", "vmap")):
                accepts.add(key)
        for key, policy in chains.items():
            bare = first.get(key)
            if (policy == "drop" and key not in accepts) or (bare and len(bare) == 1 and "drop" in bare[0]):
                out["nft"] = True
    filters = run_json("tc", "-j", "filter", "show", "dev", name, "egress")
    if isinstance(filters, dict):
        out["tc"] = filters
    else:
        out["tc"] = False
        real = [f for f in filters if isinstance(f.get("options"), dict)]
        # The first filter by preference decides what every frame meets.
        real.sort(key=lambda f: f.get("pref") or 0)
        if real and real[0].get("kind") == "matchall" and real[0].get("protocol") == "all":
            acts = real[0]["options"].get("actions") or []
            if acts and (acts[0].get("control_action") or {}).get("type") == "drop":
                out["tc"] = True
    return out


def packet_sockets():
    """The interface of each packet socket a program records with. The network
    stack's own programs hold packet sockets on every port (LLDP, DHCP) and
    record nothing, so each socket is traced to its program; one whose program
    cannot be found is an error, not a recorder."""
    try:
        rows = text("/proc/net/packet").splitlines()[1:]
    except OSError as e:
        return {"error": str(e)}
    index = {}
    for p in glob.glob("/sys/class/net/*/ifindex"):
        try:
            index[int(text(p))] = os.path.basename(os.path.dirname(p))
        except (OSError, ValueError):
            pass
    owners = {}
    for fd in glob.glob("/proc/[0-9]*/fd/*"):
        try:
            m = re.fullmatch(r"socket:\[(\d+)\]", os.readlink(fd))
            if m:
                owners[m.group(1)] = text("/proc/%s/comm" % fd.split("/")[2])
        except OSError:
            pass
    out = set()
    for r in rows:
        f = r.split()
        if len(f) < 9:
            continue
        program = owners.get(f[8])
        if program is None:
            return {"error": "a packet socket on interface %s belongs to no program this could read" % f[4]}
        # comm is cut at 15 characters.
        if program in ("systemd-network", "NetworkManager", "lldpd", "dhclient", "dhcpcd", "wpa_supplicant", "hostapd"):
            continue
        out.add(index.get(int(f[4]), "any" if f[4] == "0" else f[4]))
    return sorted(out)


def senders(names):
    """Programs and network configuration that put frames of their own on a port."""
    found = []
    for d in glob.glob("/proc/[0-9]*/cmdline"):
        try:
            argv = [a for a in text(d).split("\0") if a]
        except OSError:
            continue
        if not argv:
            continue
        prog = os.path.basename(argv[0])
        if prog in ("lldpd", "lldpad", "ladvd", "radvd"):
            found.append(prog)
        if prog in ("dhclient", "dhcpcd", "udhcpc"):
            on = [n for n in names if any(n == a or a.endswith("=" + n) for a in argv)]
            if on or not any(a in os.listdir("/sys/class/net") for a in argv[1:]):
                found.append("%s (%s)" % (prog, ", ".join(on) or "every interface"))
    for name in names:
        conf = networkd(name)
        if "error" in conf:
            continue
        for key, off in (("dhcp", ("no", "false", "none", "0", "off")), ("emitlldp", ("no", "false", "0", "off")), ("ipv6acceptra", ("no", "false", "0", "off")), ("ipv6sendra", ("no", "false", "0", "off"))):
            v = conf.get(key)
            if v is not None and v.lower() not in off and not (key == "ipv6acceptra" and number("/proc/sys/net/ipv6/conf/%s/disable_ipv6" % name) == 1):
                found.append("%s on %s (systemd-networkd %s)" % (key, name, conf["file"]))
    return sorted(set(found))


def networkd(name):
    """The systemd-networkd file that matches an interface by name: the first
    in networkd's own order. Keys in lower case."""
    import fnmatch
    files = {}
    for d in ("/usr/lib/systemd/network", "/run/systemd/network", "/etc/systemd/network"):
        for f in glob.glob(d + "/*.network"):
            files[os.path.basename(f)] = f
    for base in sorted(files):
        try:
            body = text(files[base])
        except OSError as e:
            return {"error": str(e)}
        section, keys, matches = "", {}, []
        for line in body.splitlines():
            line = line.strip()
            if line.startswith("["):
                section = line.strip("[]").lower()
            elif "=" in line and not line.startswith(("#", ";")):
                k, v = (x.strip() for x in line.split("=", 1))
                if section == "match" and k.lower() == "name":
                    matches += v.split()
                elif section in ("network", "link", "dhcpv4"):
                    keys[k.lower()] = v
        if any(fnmatch.fnmatch(name, m) for m in matches):
            return dict(keys, file=files[base])
    return {"error": "no systemd-networkd file matches %s by name" % name}


def boot_waits(names):
    """Whether boot waits for a port: RequiredForOnline in its networkd file."""
    out = {}
    for name in names:
        conf = networkd(name)
        if "error" in conf:
            out[name] = conf
        else:
            out[name] = conf.get("requiredforonline", "yes").lower() not in ("no", "false", "0", "off")
    return out


def root_encrypted():
    source = run("findmnt", "-nvo", "SOURCE", "/")
    if isinstance(source, dict):
        return source
    source = source.strip()
    if not source.startswith("/dev/"):
        return {"error": "the root is %s, not a block device" % source}
    kinds = run("lsblk", "-snro", "TYPE", source)
    if isinstance(kinds, dict):
        return kinds
    return "crypt" in kinds.split()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ports", required=True)
    ap.add_argument("--management", required=True)
    a = ap.parse_args()
    ports = [p for p in a.ports.split(",") if p]
    br = bridges()
    over = [b for b, v in br.items() if set(v["ports"]) & set(ports)]
    watchdog = run("systemctl", "show", "-p", "RuntimeWatchdogUSec", "--value")
    print(json.dumps({
        "interfaces": {n: interface(n) for n in ports + over},
        "management": interface(a.management)["exists"],
        "bridges": br,
        "routes": routes(),
        "egress": {p: egress(p) for p in ports},
        "packet_sockets": packet_sockets(),
        "senders": senders(ports + over),
        "boot_waits": boot_waits(ports),
        "watchdog": {"timeout": watchdog if isinstance(watchdog, dict) else watchdog.strip(),
                     "device": os.path.exists("/dev/watchdog0") or os.path.exists("/dev/watchdog")},
        "root_encrypted": root_encrypted(),
    }))
    return 0


if __name__ == "__main__":
    sys.exit(main())
