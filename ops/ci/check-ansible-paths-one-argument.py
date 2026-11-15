#!/usr/bin/env python3
"""check-ansible-paths-one-argument.py — a path a role hands to a command stays one argument.

`ansible.builtin.command` splits its string at whitespace (shlex), and a
`shell:` redirect (`> path`, `< path`) splits it in the shell. A path templated
into either, unquoted, becomes two arguments when it holds a space: the
operator's checkout under a folder with a space, or a data path overridden to
one. This check refuses, in every task and handler file under
ops/ansible/roles/:

  - a `command:` string (or `cmd:`) that templates a path, a `{{ ... }}` naming a
    `*_path` or `*_dir` variable, `playbook_dir` or `role_path`, or joining with
    `~ '/'`, unless that expression ends in `| quote` or sits inside a quoted
    word. Use `argv:` instead.
  - a `shell:` redirect (`>`, `>>`, `<`) straight onto such an expression,
    unquoted. Quote it (`| quote`).

Sites known today are listed in ops/ci/ansible-paths-one-argument-baseline.txt,
one per line as `<file> :: <task name>`. The list only shrinks: a site not on it
fails, and a listed site that no longer matches is a stale line, refused until it
is removed.

usage: check-ansible-paths-one-argument.py [--root DIR] [--self-test]
exit 0 no new site · 1 a new site or a stale baseline line · 2 cannot run
"""
import argparse
import re
import sys
import tempfile
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("X-PY-001: check-ansible-paths-one-argument.py needs PyYAML (pip install pyyaml)")

ROOT = Path(__file__).resolve().parents[2]
BASELINE = "ops/ci/ansible-paths-one-argument-baseline.txt"
EXPR = re.compile(r"\{\{(.*?)\}\}", re.S)
PATHLIKE = re.compile(r"\b\w+_(path|dir)\b|\bplaybook_dir\b|\brole_path\b|~\s*['\"]/")
QUOTED = re.compile(r"\|\s*quote\s*\)?\s*$")


class Loader(yaml.SafeLoader):
    pass


# Task files tag literal Jinja with !unsafe; its value is all this reads.
Loader.add_constructor("!unsafe", lambda loader, node: loader.construct_scalar(node))


def tasks(node):
    if isinstance(node, list):
        for item in node:
            yield from tasks(item)
    elif isinstance(node, dict):
        yield node
        for key in ("block", "rescue", "always"):
            if key in node:
                yield from tasks(node[key])


def in_quoted_word(s, start, end):
    before, after = s[:start], s[end:]
    return any(before.count(q) % 2 == 1 and q in after for q in ('"', "'"))


def command_hits(s):
    return [m.group(1).strip() for m in EXPR.finditer(s)
            if PATHLIKE.search(m.group(1)) and not QUOTED.search(m.group(1).strip())
            and not in_quoted_word(s, m.start(), m.end())]


def shell_hits(s):
    out = []
    for m in EXPR.finditer(s):
        if not PATHLIKE.search(m.group(1)) or QUOTED.search(m.group(1).strip()):
            continue
        if re.search(r"(?<![0-9&])(>>?|<)\s*$", s[:m.start()]) and not in_quoted_word(s, m.start(), m.end()):
            out.append(m.group(1).strip())
    return out


def sites(root):
    """{"<file> :: <task name>": [expressions]} for every site found."""
    found = {}
    files = sorted(root.glob("ops/ansible/roles/*/tasks/**/*.yml")) + sorted(root.glob("ops/ansible/roles/*/handlers/*.yml"))
    for f in files:
        doc = yaml.load(f.read_text(), Loader=Loader)
        rel = f.relative_to(root).as_posix()
        for t in tasks(doc or []):
            for mods, rule, kind in ((("ansible.builtin.command", "command"), command_hits, "command"),
                                     (("ansible.builtin.shell", "shell"), shell_hits, "shell")):
                for mod in mods:
                    v = t.get(mod)
                    s = v.get("cmd") if isinstance(v, dict) else v
                    if not isinstance(s, str):
                        continue
                    hits = rule(s)
                    if hits:
                        name = t.get("name") or f"<unnamed {kind}: {' '.join(s.split())[:60]}>"
                        found.setdefault(f"{rel} :: {name}", []).extend(hits)
    return found, len(files)


def read_baseline(root):
    p = root / BASELINE
    if not p.is_file():
        return set()
    return {ln.strip() for ln in p.read_text().splitlines() if ln.strip() and not ln.lstrip().startswith("#")}


def check(root):
    found, nfiles = sites(root)
    base = read_baseline(root)
    errs = []
    for key in sorted(set(found) - base):
        errs.append(f"{key}: templates {', '.join(found[key])} into a split string; use argv: for command:, "
                    f"or | quote for a shell: redirect")
    for key in sorted(base - set(found)):
        errs.append(f"{key}: listed in {BASELINE}, and the check no longer finds it: the line is stale, remove it")
    for e in errs:
        print(f"FAIL {e}")
    if not errs:
        print(f"ok: {nfiles} task files; no new split path; {len(base)} known site(s) on {BASELINE}")
    return 1 if errs else 0


def self_test():
    import contextlib
    import io

    def run(root):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = check(root)
        return rc, buf.getvalue()

    with tempfile.TemporaryDirectory() as d:
        r = Path(d)
        tdir = r / "ops/ansible/roles/demo/tasks"
        tdir.mkdir(parents=True)
        (r / "ops/ci").mkdir(parents=True)
        (tdir / "main.yml").write_text(
            "- name: split\n  ansible.builtin.command: \"nft -f {{ demo_dir }}/ruleset.nft\"\n"
            "- block:\n    - name: split in a block\n      ansible.builtin.command:\n"
            "        cmd: \"{{ ansible_playbook_python }} {{ playbook_dir }}/../x.py\"\n"
            "- name: argv\n  ansible.builtin.command:\n    argv: [nft, -f, \"{{ demo_dir }}/ruleset.nft\"]\n"
            "- name: quoted filter\n  ansible.builtin.command: \"nft -f {{ demo_path | quote }}\"\n"
            "- name: quoted word\n  ansible.builtin.command: sysctl -p \"{{ demo_path }}\"\n"
            "- name: not a path\n  ansible.builtin.command: \"wg show {{ demo_interface }} public-key\"\n"
            "- name: redirect\n  ansible.builtin.shell: \"umask 077; wg genkey > {{ demo_key_path }}\"\n"
            "- name: redirect quoted\n  ansible.builtin.shell: \"wg pubkey < {{ demo_key_path | quote }}\"\n"
            "- name: not a redirect\n  ansible.builtin.shell: \"echo {{ demo_path }} 2>/dev/null\"\n")
        rc, out = run(r)
        assert rc == 1, out
        for name in ("split", "split in a block", "redirect"):
            assert f"demo/tasks/main.yml :: {name}: templates" in out, (name, out)
        for name in ("argv", "quoted filter", "quoted word", "not a path", "redirect quoted", "not a redirect"):
            assert f":: {name}:" not in out, (name, out)
        (r / BASELINE).write_text("# known\n" + "".join(
            f"ops/ansible/roles/demo/tasks/main.yml :: {n}\n" for n in ("split", "split in a block", "redirect")))
        rc, out = run(r)
        assert rc == 0 and "3 known site(s)" in out, out
        # Fixing a listed site makes its line stale: refused until it is removed.
        text = (tdir / "main.yml").read_text().replace("wg genkey > {{ demo_key_path }}", "wg genkey > {{ demo_key_path | quote }}")
        (tdir / "main.yml").write_text(text)
        rc, out = run(r)
        assert rc == 1 and "redirect: listed in" in out and "stale" in out, out
        # A new site fails even with a baseline.
        (r / BASELINE).write_text("".join(f"ops/ansible/roles/demo/tasks/main.yml :: {n}\n" for n in ("split", "split in a block")))
        (tdir / "new.yml").write_text("- name: new\n  ansible.builtin.command: \"cat {{ demo_dir }}/x\"\n")
        rc, out = run(r)
        assert rc == 1 and "demo/tasks/new.yml :: new: templates" in out, out
    print("self-test ok: a split command path and an unquoted shell redirect are refused; argv, | quote, a quoted "
          "word and a non-path pass; the baseline lets a known site pass, refuses a new one and a stale line")
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    try:
        return self_test() if a.self_test else check(Path(a.root))
    except (OSError, yaml.YAMLError) as e:
        print(f"cannot run: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
