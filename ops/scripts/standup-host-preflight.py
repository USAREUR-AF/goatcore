#!/usr/bin/env python3
"""standup-host-preflight — MEASURE the target hosts before a standup touches them.

The companion to standup-config-preflight.py, and deliberately the other half of
it: that one reads group_vars and contacts no host, this one contacts hosts and
reads no group_vars. Together they are phase 1.

WHY THIS EXISTS. Standup's phase 1 checked that the configuration was complete
and never checked that the machine matched what the configuration assumes. Every
invariant below is one this repository already documents, that currently fails
LATE — during a play, or after deployment, or not at all — and each is cheap to
read up front:

  F-093  Docker AND containerd data-roots both on the Tier-3 NVMe. One of the two
         left on the root disk fills it, and the symptom is a wedged node.
  F-182  net.ipv4.ip_nonlocal_bind and the v6 equivalent = 1. Without them a VIP
         cannot be bound before it is held, and failover fails in a way that
         looks like a service crash.
  F-096  chrony-presync.service ordered BEFORE chrony and docker. Wrong order and
         containers start against an unsynced clock; certificates then fail
         validity checks that have nothing to do with certificates.

It never fixes anything and never writes. It measures, and it names what it
could not read rather than assuming the answer — an unreadable probe is an
unknown, and reporting an unknown as a pass is the substitution this whole
phase exists to prevent.

Usage:
  standup-host-preflight.py <host> [<host>...] [--user ubuntu] [--strict]

  --strict  advisory findings (arch/OS drift) fail the run too. Default is that
            only the invariants above fail; drift is reported and does not block,
            because a deliberate exception is a decision, not a defect.
  --baseline-pending
            phase 2 of this standup (mgmt-host-baseline.yml) is still ahead of
            these hosts. What that phase itself sets — the non-local-bind sysctls,
            chrony-presync, the container engine — is then reported as "set by
            phase 2 of this run" when it is not in place yet, the way the inputs
            gate reports a secret phase 3 generates. On a resume from phase 3 or
            later the same reading is an invariant failure again. The driver
            passes this when its --from is 1 or 2.
  --disk-encryption required|declared-off
            declared-off: the site declares no disk encryption
            (luks_volumes_mgmt_host: [] — goatlite's disk_encryption: false, a VM).
            An unencrypted root is then reported as the declared exclusion, not
            refused. The driver reads the declaration from the site's vars.yml.

Exit 0 all invariants hold; 1 otherwise.
"""
import argparse
import getpass
import re
import shlex
import subprocess
import sys
from pathlib import Path

OK, BAD, WARN, UNK, PEND = "[✓]", "[✗]", "[!]", "[?]", "[~]"


class Host:
    def __init__(self, addr, user, args=None):
        self.addr, self.user = addr, user
        # Extra ssh -o arguments (the transport: a door ProxyJump/ProxyCommand),
        # resolved from the same inventory ansible uses. Empty for a host reached
        # by a bare, DNS-resolvable name.
        self.args = args or []
        self.fails, self.warns, self.unknowns, self.pending = [], [], [], []
        # What the driver already knows about this run (see the flags above).
        self.baseline_pending = False
        self.disk_encryption = "required"

    def sh(self, cmd):
        """Run a command on the host. Returns (ok, output). A transport failure
        is distinct from a command that ran and said no — the caller must be able
        to tell an unknown from a negative."""
        try:
            p = subprocess.run(
                ["ssh", "-o", "StrictHostKeyChecking=no",
                 "-o", "UserKnownHostsFile=/dev/null",
                 # Without this, ssh's own "Permanently added ... to the list of
                 # known hosts" lands on stderr and gets read back as if it were
                 # the command's answer.
                 "-o", "LogLevel=ERROR",
                 "-o", "ConnectTimeout=10", "-n",
                 *self.args,
                 f"{self.user}@{self.addr}", cmd],
                capture_output=True, text=True, timeout=45)
            # stdout ONLY on success. Falling back to stderr when stdout is empty
            # turns "the command ran and printed nothing" into whatever the
            # transport happened to say, which is how a blank answer becomes a
            # confident wrong one.
            if p.returncode == 0:
                return True, p.stdout.strip()
            return False, (p.stderr or p.stdout).strip()
        except Exception as e:  # noqa: BLE001
            return False, f"{type(e).__name__}: {e}"

    def ok(self, label, detail=""):
        print(f"  {OK} {label}" + (f" — {detail}" if detail else ""))

    def bad(self, label, detail, why=""):
        print(f"  {BAD} {label} — {detail}")
        if why:
            print(f"      {why}")
        self.fails.append(label)

    def warn(self, label, detail, why=""):
        print(f"  {WARN} {label} — {detail}")
        if why:
            print(f"      {why}")
        self.warns.append(label)

    def unknown(self, label, detail):
        print(f"  {UNK} {label} — could not read: {detail}")
        self.unknowns.append(label)

    def set_by_phase_2(self, label, detail):
        """Not in place yet, and phase 2 of this run is what puts it there. Counted
        apart from failures and unknowns: it is neither wrong nor unread, it is
        still ahead."""
        print(f"  {PEND} {label} — {detail}; set by phase 2 of this run")
        self.pending.append(label)


def _root_is_sole_data_volume(h):
    """True when root is the node's only large data-bearing filesystem, so
    Tier-3 legitimately lives on root because there is no separate NVMe/LV to
    move container data to: the single-encrypted-volume node-class
    (host-site-node-disk-encryption.md §6/§82/§122, the Pi dropbear-reencrypt
    plain-partition shape) and a goatlite VM's one plain disk.

    The test is size only: no OTHER mounted real filesystem is anywhere near
    root's size. If a bigger data disk exists and the runtimes are not on it,
    both-on-root IS the F-093 failure, so this returns False and the caller
    fails the node. Whether root must be encrypted is the "disk encryption"
    row's question, graded against the site's declaration; this row does not
    repeat it."""
    ok, out = h.sh("df -B1 --output=size,target -x tmpfs -x devtmpfs "
                   "-x squashfs -x overlay 2>/dev/null | tail -n +2")
    if not ok or not out:
        return False
    root_size, other_max = 0, 0
    for line in out.splitlines():
        parts = line.split(None, 1)
        if len(parts) != 2 or not parts[0].isdigit():
            continue
        size, tgt = int(parts[0]), parts[1].strip()
        if tgt == "/":
            root_size = size
        elif tgt.startswith(("/boot", "/snap", "/run", "/dev", "/sys", "/proc", "/var/lib/docker")):
            continue
        else:
            other_max = max(other_max, size)
    # Root is the sole large store when nothing else mounted reaches half its size.
    return root_size > 0 and other_max < root_size // 2


def check(h):
    print(f"\n── {h.addr}")
    reachable, out = h.sh("echo up")
    if not reachable:
        h.bad("reachable", out, "Nothing below could be measured on this host.")
        return

    # --- what this machine is -------------------------------------------------
    ok, osrel = h.sh(". /etc/os-release && echo \"$ID $VERSION_ID\"")
    ok2, arch = h.sh("uname -m")
    if ok and ok2:
        h.ok("platform", f"{osrel} {arch}")
        # The fleet baseline: x86 on Ubuntu 24.04, Jetson on UFT 22.04. Drift is
        # reported, not refused — an exception someone chose is not a defect.
        if arch == "x86_64" and not osrel.startswith("ubuntu 24.04"):
            h.warn("platform drift", f"x86 baseline is Ubuntu 24.04, this is {osrel}",
                   "26.04 has no fips-updates or usg; 22.04 misses newer role assumptions.")
    else:
        h.unknown("platform", osrel or arch)

    # --- disk encryption: root must sit on a LUKS device (first-custody) ------
    # bare-metal-install.md's contract is full-disk LUKS before anything else.
    # Pi Imager and a plain server install both leave root UNENCRYPTED, and the
    # LUKS role only enrolls an unlock mode onto an EXISTING crypto_LUKS
    # partition — it cannot create one — so an unencrypted node fails silently
    # far downstream. Catch it here. `lsblk -s` walks the device's dependency
    # chain toward the physical disk; a `crypt` node in that chain means root is
    # on dm-crypt/LUKS. (Measured: uk Karbons show crypt, the Pis do
    # not — Pi Imager produced a plaintext install.)
    ok, root_src = h.sh("findmnt -no SOURCE /")
    if ok and root_src:
        ok2, chain = h.sh(f"lsblk -s -no TYPE {shlex.quote(root_src)} 2>/dev/null")
        types = chain.split() if ok2 else []
        if ok2 and "crypt" in types:
            h.ok("disk encryption", f"root {root_src} on LUKS ({'/'.join(types)})")
        elif ok2 and h.disk_encryption == "declared-off":
            h.warn("disk encryption",
                   f"root {root_src} is not encrypted (chain: {'/'.join(types) or '?'}); "
                   "the site declares disk encryption off",
                   "A declared exclusion (luks_volumes_mgmt_host: []), the VM shape of "
                   "goatlite.md §5. Never for production: a production host is "
                   "installed with full-disk LUKS before the standup (bare-metal-install.md).")
        elif ok2:
            h.bad("disk encryption",
                  f"root {root_src} is NOT encrypted (chain: {'/'.join(types) or '?'})",
                  "Full-disk LUKS is the first-custody contract (bare-metal-install.md). "
                  "The LUKS role enrolls an unlock mode onto an EXISTING crypto_LUKS "
                  "partition; it cannot create one. Re-install encrypted before the standup.")
        else:
            h.unknown("disk encryption", chain)
    else:
        h.unknown("disk encryption", root_src or "could not read root source")

    # --- auth contract: NOPASSWD sudo (key-auth is implied by reachability) ---
    # A node keyed on one host but not its sibling is a real first-custody miss
    # (measured: one uk node was reachable, another rejected the key);
    # without this it surfaces only when the first `become` task fails mid-play.
    ok, _ = h.sh("sudo -n true")
    if ok:
        h.ok("auth (key + NOPASSWD sudo)", "sudo -n works")
    else:
        h.bad("auth (key + NOPASSWD sudo)", "sudo -n failed on this node",
              "seed-ssh-key.sh has not established NOPASSWD sudo here. Run it "
              "(bare-metal-install.md) on EVERY node before the standup.")

    # --- F-093: BOTH container runtimes on the Tier-3 NVMe --------------------
    # Try without sudo first: on a host where the operator is in the docker
    # group the answer is the same, and a needless sudo turns a readable probe
    # into an unknown wherever sudo is password-gated.
    ok, droot = h.sh("docker info --format '{{.DockerRootDir}}' 2>/dev/null")
    if not ok or not droot:
        ok, droot = h.sh("sudo -n docker info --format '{{.DockerRootDir}}' 2>/dev/null")
    ok2, croot = h.sh(
        "sudo -n grep -oE '^\\s*root\\s*=\\s*\"[^\"]+\"' /etc/containerd/config.toml 2>/dev/null "
        "| head -1 | sed 's/.*\"\\(.*\\)\"/\\1/'")
    if not ok and h.baseline_pending:
        h.set_by_phase_2("container data-roots", "no container engine on this host yet")
    elif not ok:
        h.unknown("docker data-root", droot)
    else:
        _, mnt_d = h.sh(f"df --output=target {shlex.quote(droot)} 2>/dev/null | tail -1")
        _, mnt_c = h.sh(f"df --output=target {shlex.quote(croot)} 2>/dev/null | tail -1") if croot else (False, "")
        if croot and mnt_d and mnt_c and mnt_d == mnt_c and mnt_d != "/":
            h.ok("container data-roots", f"both on {mnt_d}")
        elif croot and mnt_d and mnt_c and mnt_d == mnt_c and mnt_d == "/":
            # Both runtimes together on root. On a node with a SEPARATE Tier-3
            # data disk this is the F-093 failure — a small root fills and wedges
            # while the big disk sits unused. On a single-encrypted-volume node
            # it is correct: the Pi in-place dropbear-reencrypt shape
            # (host-site-node-disk-encryption.md §6, §122) is a plain-partition
            # LUKS root that IS the box's only large disk, with no volume group
            # to carve a docker-data LV from — so Tier-3 legitimately lives as
            # /opt/docker-data on the one encrypted volume (design §82: "a
            # legitimate variation on the three-tier model"). Distinguish the two
            # by asking whether a larger data disk sits unused.
            if _root_is_sole_data_volume(h):
                h.ok("container data-roots",
                     "both on / — root is the only data filesystem, so Tier-3 is "
                     "root; no separate data disk (design §82 variation)")
            else:
                h.bad("container data-roots",
                      "both on / while a larger data filesystem sits unused",
                      "One of the two left on the root disk fills it, and the node "
                      "wedges. Both belong on the Tier-3 NVMe that is mounted here.")
        elif mnt_d == "/" or (mnt_c == "/" and croot):
            h.bad("container data-roots",
                  f"docker on {mnt_d or '?'}, containerd on {mnt_c or '?'}",
                  "Split across filesystems, or one stranded on root: one of the two "
                  "left on the root disk fills it, and the node wedges. Both belong "
                  "on the same Tier-3 NVMe.")
        else:
            h.warn("container data-roots",
                   f"docker {droot} -> {mnt_d or '?'}, containerd {croot or 'unset'} -> {mnt_c or '?'}",
                   "Could not confirm both are on the same non-root filesystem.")

    # --- F-182: non-local bind, both families --------------------------------
    for key in ("net.ipv4.ip_nonlocal_bind", "net.ipv6.ip_nonlocal_bind"):
        ok, val = h.sh(f"sysctl -n {key} 2>/dev/null")
        if not ok or val == "":
            h.unknown(key, val or "no value")
        elif val.strip() == "1":
            h.ok(key, "1")
        elif h.baseline_pending:
            h.set_by_phase_2(key, f"is {val.strip()}")
        else:
            h.bad(key, f"is {val.strip()}, must be 1",
                  "A VIP cannot be bound before it is held; failover then fails "
                  "in a way that reads as a service crash.")

    # --- F-096: presync ordered before chrony and docker ---------------------
    ok, unit = h.sh("systemctl cat chrony-presync.service 2>/dev/null | head -40")
    if (not ok or not unit) and h.baseline_pending:
        h.set_by_phase_2("chrony-presync", "no chrony-presync.service on this host")
    elif not ok or not unit:
        h.warn("chrony-presync", "no chrony-presync.service on this host",
               "Containers may start against an unsynced clock; certificate validity "
               "then fails for reasons that have nothing to do with certificates.")
    else:
        before = [ln for ln in unit.splitlines() if ln.strip().startswith("Before=")]
        joined = " ".join(before)
        missing = [w for w in ("chrony", "docker") if w not in joined]
        if missing:
            h.bad("chrony-presync ordering",
                  f"Before= does not cover: {', '.join(missing)}",
                  f"Read: {joined or '(no Before= line)'}")
        else:
            h.ok("chrony-presync ordering", joined.strip())

    # --- clock, because everything downstream trusts it ----------------------
    ok, skew = h.sh("chronyc tracking 2>/dev/null | awk '/System time/{print $4, $5}'")
    if ok and skew:
        h.ok("clock", f"system time offset {skew}")
    else:
        ok, ts = h.sh("date -u +%s")
        if ok:
            import time
            d = abs(int(ts) - int(time.time()))
            (h.ok if d <= 5 else h.warn)("clock", f"{d}s from this machine")
        else:
            h.unknown("clock", ts)

    # --- room to work --------------------------------------------------------
    ok, free = h.sh("df -BG --output=avail / | tail -1 | tr -dc '0-9'")
    if ok and free.isdigit():
        (h.ok if int(free) >= 20 else h.warn)("root filesystem", f"{free} GiB free")
    else:
        h.unknown("root filesystem", free)


def load_inventory(files):
    """name -> (addr, user, extra_ssh_args), parsed from ansible ini inventories.

    Reaches a host the way ansible does and from the same source of truth:
    ansible_host is the address, ansible_user the login, and
    ansible_ssh_common_args carries the transport — the env-door's
    ProxyJump/ProxyCommand into an off-profile goatnet. Without this the phase-1
    preflight sshed to the bare inventory NAME (`ssh delta-core-a`), which resolves
    to nothing off the mesh and fails as "could not resolve hostname" even
    though every later ansible play reaches the same host fine. A later file
    wins, exactly as ansible layers a mesh-override inventory after the base one.

    Only these three connection keys are read; no group_vars are touched, so
    this stays the half of phase 1 that measures hosts and reads no config.
    shlex.split handles both inventory forms: the plain ProxyJump string in
    mgmt-hosts-mesh.ini and the nested-quote ProxyCommand the door generator
    emits."""
    hosts = {}
    for f in files or []:
        p = Path(f)
        if not p.is_file():
            continue
        for line in p.read_text().splitlines():
            s = line.strip()
            if not s or s.startswith("#") or s.startswith("["):
                continue
            try:
                toks = shlex.split(s)
            except ValueError:
                toks = s.split()
            if not toks or not re.fullmatch(r"[A-Za-z0-9_.:-]+", toks[0]):
                continue
            name = toks[0]
            kv = dict(t.split("=", 1) for t in toks[1:] if "=" in t)
            if "ansible_host" not in kv and name not in hosts:
                continue  # a bare host line with no address is not a transport row
            raw = kv.get("ansible_ssh_common_args", "")
            try:
                args = shlex.split(raw) if raw else []
            except ValueError:
                args = raw.split()
            hosts[name] = (kv.get("ansible_host"), kv.get("ansible_user"), args)
    return hosts


def default_user() -> str:
    """The account this project reaches a machine as: deploy.ssh_user in
    ops/goatbuilder/goatbuilder.yaml, else the login name, which is what ssh
    itself uses."""
    try:
        lines = (Path(__file__).resolve().parents[2] / "ops/goatbuilder/goatbuilder.yaml").read_text().splitlines()
    except OSError:
        lines = []
    inside = False
    for line in lines:
        if line[:1] not in ("", " ", "\t", "#"):
            inside = line.rstrip() == "deploy:"
        elif inside and line.strip().startswith("ssh_user:"):
            value = line.split(":", 1)[1].split("#", 1)[0].strip().strip("'\"")
            if value:
                return value
    return getpass.getuser()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("hosts", nargs="+")
    ap.add_argument("--user", default=default_user(),
                    help="ssh user for a host the inventory names no ansible_user for "
                         "(default: deploy.ssh_user in ops/goatbuilder/goatbuilder.yaml, else the login name)")
    ap.add_argument("--inventory", action="append", default=None,
                    help="ansible inventory file(s) that carry the host transport "
                         "(ansible_host / ansible_user / ansible_ssh_common_args); "
                         "repeatable, later files win. Resolves each host the way "
                         "ansible does, including the env-door ProxyJump. Without "
                         "it, a host is sshed by its bare name and must resolve via "
                         "DNS or ssh_config.")
    ap.add_argument("--strict", action="store_true")
    ap.add_argument("--baseline-pending", action="store_true",
                    help="phase 2 of this standup is still ahead of these hosts: what it "
                         "sets is reported as pending, not failed, when absent")
    ap.add_argument("--disk-encryption", choices=["required", "declared-off"], default="required",
                    help="declared-off: the site declares no disk encryption "
                         "(luks_volumes_mgmt_host: []); an unencrypted root is the "
                         "declared exclusion, not a refusal")
    a = ap.parse_args()

    print("standup-host-preflight — measuring, changing nothing")
    inv = load_inventory(a.inventory)
    hs, unresolved = [], []
    for name in a.hosts:
        if name in inv:
            addr, user, args = inv[name]
            if not addr or str(addr).startswith("TBD_"):
                unresolved.append(name)
                continue
            hs.append(Host(addr, user or a.user, args))
        else:
            # Not in any given inventory: fall back to the bare name (a founder
            # node reachable by DNS/ssh_config). Keeps this usable with no
            # --inventory at all.
            hs.append(Host(name, a.user))
    for name in unresolved:
        print(f"\n── {name}")
        print(f"  {BAD} reachable — no usable address in the given inventory "
              "(ansible_host missing or still a TBD_ sentinel).")
        print("      The door/mesh inventory must carry this host's ansible_host "
              "before the standup can measure it.")
    for h in hs:
        h.baseline_pending = a.baseline_pending
        h.disk_encryption = a.disk_encryption
        check(h)

    fails = sum(len(h.fails) for h in hs) + len(unresolved)
    warns = sum(len(h.warns) for h in hs)
    unk = sum(len(h.unknowns) for h in hs)
    pend = sum(len(h.pending) for h in hs)
    print(f"\n{fails} invariant failure(s), {warns} advisory, {unk} unreadable"
          + (f", {pend} set by phase 2 of this run" if pend else "") + ".")
    if unk:
        print("An unreadable probe is an UNKNOWN, never a pass. Resolve those before "
              "treating this host as measured.")
    if fails or (a.strict and warns):
        print("NOT READY — fix the above, then re-run. Nothing was changed.")
        return 1
    print("READY — the invariants this checks hold on every host named.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
