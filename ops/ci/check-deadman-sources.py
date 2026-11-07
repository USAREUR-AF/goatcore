#!/usr/bin/env python3
"""Refuse an obs host whose dead-man heartbeat another obs host can mask.

The dead-man's switch has two halves: each obs host's Alertmanager posts its
always-firing Watchdog alert to a watcher on another machine
(`roles/deadman-watch`), and the watcher pages when the heartbeat stops. The
watcher must not keep ONE timestamp for everything that posts to it: with two
obs hosts posting to one watcher, a live heartbeat from one keeps the timestamp
fresh while the other is dead, and nothing pages.

The watcher now keeps one timestamp per request path and judges every host in
`deadman_watch_expected_sources` separately. That only helps if both ends are
wired, so this guard checks, for every group_vars directory that declares
`deadman_webhook_url`:

  1. every watcher URL in it (anything on the watcher port, :9099) ends in the
     sending host's own name — `/{{ inventory_hostname }}` or
     `/' ~ inventory_hostname`. Without it the host writes the shared legacy
     timestamp and can mask, or be masked by, any other sender.
  2. when the URL names a literal address, and an inventory host at that
     address belongs to a group that sets `deadman_watch_enabled`, every
     inventory host of the obs group is listed in that group's
     `deadman_watch_expected_sources`. A sender the watcher does not expect is
     a heartbeat nobody judges.

  3. a watcher-port URL whose address belongs to no group that sets
     `deadman_watch_enabled` FAILS: the heartbeat goes nowhere, so the site's
     death pages nobody (an obs group posting to a host where no watcher is
     deployed).

A third-party absence detector (DEADMAN_WEBHOOK_URL / deadman_external_url) is
one check per URL and is not on the watcher port, so neither rule touches it.

Usage:
  ops/ci/check-deadman-sources.py [<ansible-root>]   # verdict (default ops/ansible)
  ops/ci/check-deadman-sources.py --self-test         # prove RED on fixtures (P6)
"""
import re
import sys
import tempfile
from pathlib import Path

import yaml

WATCHER_PORT = "9099"
URL_ON_PORT = re.compile(r":" + WATCHER_PORT + r"/")
PER_HOST = re.compile(
    r":" + WATCHER_PORT + r"/(\{\{\s*inventory_hostname\s*\}\}|'\s*~\s*inventory_hostname)"
)
IPV4 = re.compile(r"\b(\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3})\b")


def load_group_vars(root: Path) -> dict:
    groups = {}
    gv = root / "group_vars"
    if not gv.is_dir():
        return groups
    for d in sorted(p for p in gv.iterdir() if p.is_dir()):
        merged = {}
        for f in sorted(d.glob("*.yml")) + sorted(d.glob("*.yaml")):
            try:
                data = yaml.safe_load(f.read_text())
            except yaml.YAMLError as exc:
                print(f"WARN: {f}: unparseable, skipped ({exc.__class__.__name__})")
                continue
            if isinstance(data, dict):
                merged.update(data)
        groups[d.name] = merged
    return groups


def load_inventory(root: Path):
    """Return ({group: set(hosts)}, {ip: set(hosts)}) from every inventory/*.ini."""
    members, by_ip = {}, {}
    for ini in sorted((root / "inventory").glob("*.ini")):
        section = None
        for raw in ini.read_text().splitlines():
            line = raw.strip()
            if not line or line.startswith(("#", ";")):
                continue
            m = re.match(r"^\[([^\]]+)\]$", line)
            if m:
                name = m.group(1)
                section = None if ":" in name else name
                continue
            if section is None:
                continue
            host = line.split()[0]
            members.setdefault(section, set()).add(host)
            # A host reached over SSM carries its instance id in ansible_host and
            # its mesh address in mesh_ip_a, which is what a heartbeat URL names.
            for key in ("ansible_host", "mesh_ip_a"):
                ah = re.search(r"\b%s=(\S+)" % key, line)
                if ah:
                    by_ip.setdefault(ah.group(1), set()).add(host)
    return members, by_ip


def check(root: Path) -> int:
    groups = load_group_vars(root)
    members, by_ip = load_inventory(root)
    watcher_groups = {g for g, v in groups.items() if "deadman_watch_enabled" in v}
    failures, warnings = [], []

    for g, v in sorted(groups.items()):
        url = v.get("deadman_webhook_url")
        if not isinstance(url, str) or not URL_ON_PORT.search(url):
            continue
        # Rule 1 — per-host path on every watcher-port URL in the value.
        on_port = len(URL_ON_PORT.findall(url))
        if len(PER_HOST.findall(url)) < on_port:
            failures.append(
                f"{g}: deadman_webhook_url posts to a watcher without a per-host path "
                f"(needs ':{WATCHER_PORT}/{{{{ inventory_hostname }}}}'): {url.strip()}"
            )
        # Rule 2 — the watcher at a literal address expects every sender.
        senders = members.get(g, set())
        for ip in IPV4.findall(url):
            if ip.startswith("127."):
                continue
            hosts = by_ip.get(ip, set())
            owning = sorted(
                wg for wg in watcher_groups if hosts & members.get(wg, set())
            )
            if not owning:
                failures.append(
                    f"{g}: deadman_webhook_url names {ip}:{WATCHER_PORT}, and no group that "
                    f"sets deadman_watch_enabled has a host at that address — "
                    f"the heartbeat reaches no watcher, so the site's death pages nobody"
                )
                continue
            for wg in owning:
                expected = set(groups[wg].get("deadman_watch_expected_sources") or [])
                missing = sorted(senders - expected)
                if missing:
                    failures.append(
                        f"{g}: posts to the {wg} watcher at {ip}, whose "
                        f"deadman_watch_expected_sources does not list {', '.join(missing)} — "
                        f"a heartbeat nobody judges"
                    )

    for w in warnings:
        print(f"WARN: {w}")
    for f in failures:
        print(f"FAIL: {f}")
    if failures:
        print(f"check-deadman-sources: {len(failures)} failure(s) — see F-529")
        return 1
    print("OK: every obs host posts its dead-man heartbeat on its own path, and its watcher expects it")
    return 0


def write(root: Path, rel: str, text: str) -> None:
    p = root / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text)


def fixture(root: Path, obs_url: str, expected: str) -> None:
    write(root, "inventory/hosts.ini",
          "[site_hosts]\nwatcher-a ansible_host=100.127.0.1\n"
          "[site_obs_hosts]\nobs-1 ansible_host=100.127.0.3\n"
          "[site_ltu_obs_hosts]\nobs-2 ansible_host=100.127.0.4\n")
    write(root, "group_vars/site_hosts/vars.yml",
          "deadman_watch_enabled: true\n" + expected)
    for g in ("site_obs_hosts", "site_ltu_obs_hosts"):
        write(root, f"group_vars/{g}/vars.yml", f'deadman_webhook_url: "{obs_url}"\n')


def self_test() -> int:
    per_host = "http://{{ mesh_ip_a | default('100.127.0.1') }}:9099/{{ inventory_hostname }}"
    shared = "http://{{ mesh_ip_a | default('100.127.0.1') }}:9099/"
    both = "deadman_watch_expected_sources: [obs-1, obs-2]\n"
    cases = [
        ("RED: two obs groups share the watcher's single timestamp (the F-529 state)", shared, both, 1),
        ("RED: per-host paths, but the watcher does not expect obs-2", per_host,
         "deadman_watch_expected_sources: [obs-1]\n", 1),
        ("RED: per-host paths, watcher expects nobody", per_host, "", 1),
        ("RED: per-host paths to an address where no watcher is enabled",
         "http://100.127.0.7:9099/{{ inventory_hostname }}", both, 1),
        ("GREEN: per-host paths and both senders expected", per_host, both, 0),
    ]
    for name, url, expected, want in cases:
        with tempfile.TemporaryDirectory() as t:
            root = Path(t)
            fixture(root, url, expected)
            got = check(root)
        if got != want:
            print(f"SELF-TEST FAIL: {name} — exit {got}, wanted {want}")
            return 1
        print(f"self-test: {name} — proven")
    print("OK: check-deadman-sources self-test — red-first proof holds")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        sys.exit(self_test())
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2] / "ops" / "ansible"
    sys.exit(check(root))
