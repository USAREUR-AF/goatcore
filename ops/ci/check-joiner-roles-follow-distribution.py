#!/usr/bin/env python3
"""check-joiner-roles-follow-distribution.py — the apt sources a joiner's roles add
follow the host's distribution (gate row H6: the plays a joiner runs work on
Debian as on Ubuntu).

A joining site's hosts may run Debian or Ubuntu. A role that adds an apt
repository or its key with a literal distribution in the URL or the line
(`/ubuntu/`, `noble`, `jammy`, `focal`) points a Debian host at Ubuntu's
packages: the download fails, or installs the wrong build. The distribution and
its codename come from the facts: `{{ ansible_distribution | lower }}` and
`{{ ansible_distribution_release }}`.

Which roles: those the joiner's plays reach, walked from the plays `goatbuilder
join-node` and a joiner's `standup` run (PLAYS below): every role named in
`roles:`, `include_role`/`import_role` and `meta/main.yml` dependencies, through
`import_playbook`, recursively.

What it refuses, in those roles' task files: a literal `ubuntu` path segment or
an Ubuntu codename in an apt source, written by
  - ansible.builtin.apt_repository `repo:`,
  - ansible.builtin.apt_key `url:`,
  - ansible.builtin.deb822_repository `uris:`/`suites:`,
  - ansible.builtin.get_url `url:` whose `dest:` is under /etc/apt/ or a keyring
    directory, and
  - ansible.builtin.copy `content:` whose `dest:` is under /etc/apt/,
unless that part is a Jinja expression. A host name such as `ports.ubuntu.com`
used for something other than an apt source (a time server) is not an apt source
and is not read.

usage: check-joiner-roles-follow-distribution.py [--root DIR] [--self-test]
exit 0 every apt source follows the distribution · 1 a literal one · 2 cannot run
"""
import argparse
import re
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("X-PY-001: check-joiner-roles-follow-distribution.py needs PyYAML (pip install pyyaml)")

ROOT = Path(__file__).resolve().parents[2]
# The plays a joiner's hosts run: crates/goatbuilder/src/joinnode.rs and the
# joiner branch of crates/goatbuilder/src/standup.rs.
PLAYS = (
    "mgmt-host-baseline", "wireguard-kmod", "mesh-enroll-host", "site-bootstrap", "site-bootstrap-for",
    "ory-deploy", "site-seed-from-peer", "spock-mesh-wire", "stage-service-leaves", "zenoh-deploy",
    "traefik-acme", "lan-traefik-routes", "signal-federation-transport", "signal-federation-verify",
    "relay-ingress-add-site", "site-rathole-client", "goat-coredns", "fabric-health-deploy",
    "mesh-health-deploy", "presence-health-deploy", "time-health-deploy", "consistency-health-deploy",
    "fabric-crawler-deploy", "utility-host-bootstrap", "obs-host-bootstrap", "mgmt-host-luks",
    "tang-server", "breakglass-nodes", "herd-connector-deploy",
)
LITERAL = re.compile(r"(?:/ubuntu(?:/|\b)|\b(?:noble|jammy|focal)\b)", re.I)
EXPR = re.compile(r"\{\{.*?\}\}", re.S)


class Loader(yaml.SafeLoader):
    pass


Loader.add_constructor("!unsafe", lambda loader, node: loader.construct_scalar(node))
Loader.add_multi_constructor("!", lambda loader, suffix, node: None)


def load(path):
    try:
        return yaml.load(path.read_text(), Loader=Loader)
    except yaml.YAMLError:
        return None


def named_roles(node, roles, playbooks):
    if isinstance(node, list):
        for x in node:
            named_roles(x, roles, playbooks)
    elif isinstance(node, dict):
        for k, v in node.items():
            if k == "roles" and isinstance(v, list):
                for r in v:
                    n = r if isinstance(r, str) else (r.get("role") or r.get("name")) if isinstance(r, dict) else None
                    if isinstance(n, str) and "{{" not in n:
                        roles.add(n)
            elif k in ("include_role", "import_role", "ansible.builtin.include_role",
                       "ansible.builtin.import_role") and isinstance(v, dict):
                n = v.get("name")
                if isinstance(n, str) and "{{" not in n:
                    roles.add(n)
            elif k in ("import_playbook", "ansible.builtin.import_playbook") and isinstance(v, str) and "{{" not in v:
                playbooks.add(v)
            named_roles(v, roles, playbooks)


def joiner_roles(root):
    a = root / "ops/ansible"
    roles, seen, todo = set(), set(), [a / f"{p}.yml" for p in PLAYS]
    while todo:
        f = todo.pop()
        if f in seen or not f.is_file():
            continue
        seen.add(f)
        books = set()
        named_roles(load(f) or [], roles, books)
        todo += [f.parent / b for b in books]
    done, queue = set(), sorted(roles)
    while queue:
        r = queue.pop()
        if r in done or not (a / "roles" / r).is_dir():
            continue
        done.add(r)
        more = set()
        for f in sorted((a / "roles" / r).glob("tasks/**/*.yml")):
            named_roles(load(f) or [], more, set())
        meta = load(a / "roles" / r / "meta/main.yml") if (a / "roles" / r / "meta/main.yml").is_file() else None
        if isinstance(meta, dict):
            for dep in meta.get("dependencies") or []:
                n = dep if isinstance(dep, str) else dep.get("role") if isinstance(dep, dict) else None
                if isinstance(n, str):
                    more.add(n)
        queue += sorted(more - done)
    return sorted(done)


def tasks(node):
    if isinstance(node, list):
        for item in node:
            yield from tasks(item)
    elif isinstance(node, dict):
        yield node
        for key in ("block", "rescue", "always"):
            if key in node:
                yield from tasks(node[key])


def literal(value):
    """The literal distribution parts of a string, with Jinja expressions removed."""
    if isinstance(value, list):
        return [m for v in value for m in literal(v)]
    if not isinstance(value, str):
        return []
    return [m.group(0) for m in LITERAL.finditer(EXPR.sub(" ", value))]


def module(task, *names):
    for n in names:
        for key in (f"ansible.builtin.{n}", n):
            if isinstance(task.get(key), dict):
                return task[key]
    return None


def apt_dest(dest):
    return isinstance(dest, str) and (dest.startswith("/etc/apt/") or "keyring" in dest)


def sites(root):
    found, roles = [], joiner_roles(root)
    for r in roles:
        for f in sorted((root / "ops/ansible/roles" / r).glob("tasks/**/*.yml")):
            rel = f.relative_to(root).as_posix()
            for t in tasks(load(f) or []):
                hits = []
                m = module(t, "apt_repository")
                if m:
                    hits += literal(m.get("repo"))
                m = module(t, "apt_key")
                if m:
                    hits += literal(m.get("url"))
                m = module(t, "deb822_repository")
                if m:
                    hits += literal(m.get("uris")) + literal(m.get("suites"))
                m = module(t, "get_url")
                if m and apt_dest(m.get("dest")):
                    hits += literal(m.get("url"))
                m = module(t, "copy")
                if m and isinstance(m.get("dest"), str) and m["dest"].startswith("/etc/apt/"):
                    hits += literal(m.get("content"))
                if hits:
                    found.append(f"{rel} :: {t.get('name') or '<unnamed>'}: {', '.join(sorted(set(hits)))}")
    return found, roles


def check(root):
    found, roles = sites(root)
    for s in found:
        print(f"FAIL {s} is a literal distribution in an apt source; use "
              "{{ ansible_distribution | lower }} and {{ ansible_distribution_release }}")
    if not found:
        print(f"ok: {len(roles)} roles the joiner's plays reach; every apt source follows the distribution")
    return 1 if found else 0


def self_test():
    import contextlib
    import io

    with tempfile.TemporaryDirectory() as d:
        r = Path(d)
        a = r / "ops/ansible"
        (a / "roles/joined/tasks").mkdir(parents=True)
        (a / "roles/joined/meta").mkdir(parents=True)
        (a / "roles/dep/tasks").mkdir(parents=True)
        (a / "roles/elsewhere/tasks").mkdir(parents=True)
        (a / "mgmt-host-baseline.yml").write_text("- hosts: all\n  roles:\n    - joined\n")
        (a / "roles/joined/meta/main.yml").write_text("dependencies:\n  - role: dep\n")
        (a / "roles/joined/tasks/main.yml").write_text(
            "- name: key literal\n  ansible.builtin.get_url:\n"
            "    url: https://pkgs.example.org/stable/ubuntu/{{ ansible_distribution_release }}.gpg\n"
            "    dest: /usr/share/keyrings/example.gpg\n"
            "- name: repo literal codename\n  ansible.builtin.apt_repository:\n"
            "    repo: deb https://pkgs.example.org/{{ ansible_distribution | lower }} noble main\n"
            "- name: follows\n  ansible.builtin.apt_repository:\n"
            "    repo: deb https://pkgs.example.org/{{ ansible_distribution | lower }} {{ ansible_distribution_release }} main\n"
            "- name: not an apt source\n  ansible.builtin.get_url:\n"
            "    url: https://ports.ubuntu.com/ubuntu/noble/x\n    dest: /tmp/x\n"
            "- name: include\n  ansible.builtin.include_role:\n    name: elsewhere\n")
        (a / "roles/dep/tasks/main.yml").write_text(
            "- name: deb822 literal\n  ansible.builtin.deb822_repository:\n"
            "    name: x\n    uris: https://pkgs.example.org/ubuntu\n    suites: \"{{ ansible_distribution_release }}\"\n")
        (a / "roles/elsewhere/tasks/main.yml").write_text(
            "- name: apt line literal\n  ansible.builtin.copy:\n"
            "    dest: /etc/apt/sources.list.d/x.list\n    content: \"deb https://pkgs.example.org/ubuntu jammy main\\n\"\n")
        (a / "roles/unreached/tasks").mkdir(parents=True)
        (a / "roles/unreached/tasks/main.yml").write_text(
            "- name: not a joiner role\n  ansible.builtin.apt_repository:\n    repo: deb https://x/ubuntu noble main\n")
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(r)
        out = buf.getvalue()
        assert rc == 1, out
        for name, what in (("key literal", "/ubuntu/"), ("repo literal codename", "noble"),
                           ("deb822 literal", "/ubuntu"), ("apt line literal", "jammy")):
            assert f":: {name}: " in out and what in out, (name, out)
        for name in ("follows", "not an apt source", "not a joiner role"):
            assert f":: {name}:" not in out, (name, out)
    print("self-test ok: a literal Ubuntu path or codename in an apt key, repository, deb822 source or "
          "sources.list line of a role the joiner's plays reach (by roles:, include_role, meta dependency) is "
          "refused; one that follows the facts, a non-apt download and a role no joiner play reaches pass")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    try:
        return self_test() if a.self_test else check(Path(a.root))
    except OSError as e:
        print(f"cannot run: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
