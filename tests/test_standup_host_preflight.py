"""standup-host-preflight grades a fresh box by what the driver tells it about the run.

A fresh Ubuntu VM has no container engine, non-local bind off and no presync
unit: phase 2 of the standup sets all three. Told that phase 2 is still ahead,
the preflight reports them as set by phase 2 of this run; told nothing, they
are invariant failures, as on a resume from phase 3. A site that declares no
disk encryption (goatlite's VM shape) gets an advisory for its plain root, not a
refusal.
"""
import importlib.machinery
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent / "ops" / "scripts" / "standup-host-preflight.py"


def load():
    loader = importlib.machinery.SourceFileLoader("standup_host_preflight", str(SCRIPT))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    mod = importlib.util.module_from_spec(spec)
    loader.exec_module(mod)
    return mod


FRESH_VM = [
    ("echo up", (True, "up")),
    ("/etc/os-release", (True, "ubuntu 24.04")),
    ("uname -m", (True, "x86_64")),
    ("findmnt -no SOURCE /", (True, "/dev/nvme0n1p1")),
    ("lsblk -s -no TYPE", (True, "part\ndisk")),
    ("sudo -n true", (True, "")),
    ("docker info", (False, "")),
    ("containerd/config.toml", (False, "")),
    ("sysctl -n net.ipv4.ip_nonlocal_bind", (True, "0")),
    ("sysctl -n net.ipv6.ip_nonlocal_bind", (True, "0")),
    ("systemctl cat chrony-presync.service", (False, "")),
    ("chronyc tracking", (True, "0.000000338 seconds")),
    ("df -BG --output=avail /", (True, "94")),
]


def fake_host(mod, answers, **flags):
    class Fake(mod.Host):
        def sh(self, cmd):
            for needle, answer in answers:
                if needle in cmd:
                    return answer
            return False, f"no canned answer for: {cmd}"

    h = Fake("203.0.113.9", "ubuntu")
    for k, v in flags.items():
        setattr(h, k, v)
    return h


def test_a_fresh_vm_before_phase_2_has_no_failures_once_the_run_is_declared(capsys):
    mod = load()
    h = fake_host(mod, FRESH_VM, baseline_pending=True, disk_encryption="declared-off")
    mod.check(h)
    assert h.fails == []
    assert h.unknowns == []
    assert sorted(h.pending) == sorted([
        "container data-roots",
        "net.ipv4.ip_nonlocal_bind",
        "net.ipv6.ip_nonlocal_bind",
        "chrony-presync",
    ])
    assert h.warns == ["disk encryption"]
    out = capsys.readouterr().out
    assert out.count("set by phase 2 of this run") == 4
    assert "the site declares disk encryption off" in out


def test_the_same_vm_told_nothing_fails_as_before(capsys):
    mod = load()
    h = fake_host(mod, FRESH_VM)
    mod.check(h)
    assert "disk encryption" in h.fails
    assert "net.ipv4.ip_nonlocal_bind" in h.fails
    assert "net.ipv6.ip_nonlocal_bind" in h.fails
    assert "docker data-root" in h.unknowns
    assert "chrony-presync" in h.warns
    assert h.pending == []


def test_a_baselined_host_is_read_the_same_with_or_without_the_flag():
    mod = load()
    baselined = [
        ("sysctl -n net.ipv4.ip_nonlocal_bind", (True, "1")),
        ("sysctl -n net.ipv6.ip_nonlocal_bind", (True, "1")),
        ("systemctl cat chrony-presync.service", (True, "[Unit]\nBefore=chrony.service docker.service")),
        ("docker info", (True, "/var/lib/docker")),
        ("containerd/config.toml", (True, "/var/lib/containerd")),
        ("df --output=target", (True, "/")),
        ("findmnt -no SOURCE /", (True, "/dev/mapper/root")),
        ("lsblk -s -no TYPE", (True, "crypt\npart\ndisk")),
        ("lsblk -o", (True, "")),
    ] + FRESH_VM
    for pending in (True, False):
        h = fake_host(mod, baselined, baseline_pending=pending)
        mod.check(h)
        assert h.pending == []
        assert "net.ipv4.ip_nonlocal_bind" not in h.fails
        assert "chrony-presync ordering" not in h.fails


def test_the_declaration_does_not_excuse_an_encrypted_requirement():
    mod = load()
    h = fake_host(mod, FRESH_VM, baseline_pending=True)  # disk_encryption stays "required"
    mod.check(h)
    assert "disk encryption" in h.fails


STOOD_UP_LITE = [
    ("docker info", (True, "/var/lib/docker")),
    ("containerd/config.toml", (True, "/var/lib/containerd")),
    ("df --output=target", (True, "/")),
    ("df -B1 --output=size,target", (True, "82678120448 /\n923332608 /boot\n1124999168 /boot/efi")),
] + FRESH_VM


def test_a_stood_up_goatlite_with_one_plain_disk_passes_the_data_roots_row():
    mod = load()
    h = fake_host(mod, STOOD_UP_LITE, disk_encryption="declared-off")
    mod.check(h)
    assert "container data-roots" not in h.fails
    assert h.warns.count("disk encryption") == 1


def test_a_larger_unused_data_disk_still_fails_the_data_roots_row():
    mod = load()
    bigger = [("df -B1 --output=size,target", (True, "82678120448 /\n2000398934016 /data"))] + STOOD_UP_LITE
    h = fake_host(mod, bigger, disk_encryption="declared-off")
    mod.check(h)
    assert "container data-roots" in h.fails
