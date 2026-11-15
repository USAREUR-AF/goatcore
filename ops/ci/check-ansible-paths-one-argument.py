#!/usr/bin/env python3
"""check-ansible-paths-one-argument.py — a path a role hands to a command stays one argument.

`command:` and `script:` split their string at whitespace (shlex), and
`shell:` and `raw:` split it in the shell. A path templated into any of them,
unquoted, becomes two arguments when it holds a space: the operator's checkout
under a folder with a space, or a data path overridden to one. This check reads
roles only: every task file under ops/ansible/roles/*/tasks/ and every handler
file under ops/ansible/roles/*/handlers/, at any depth. Top-level playbooks are
not read.

A path is a `{{ ... }}` naming a variable ending in `_path`, `_dir`, `_file`,
`_root` or `_prefix`, or in `dir` (`bindir`, `workdir`, `playbook_dir`), an
attribute `.path` (`item.path`), `role_path`, or joining with `~ '/'`. The check
refuses one templated into:

  - a `command:` or `script:` string (or `cmd:`), from ansible.builtin,
    ansible.legacy or the short name. Use `argv:` instead.
  - a `shell:` or `raw:` string, anywhere in it: an argument, or a redirect
    target after `>`, `>>`, `<`, `2>` or `&>`. Quote it (`| quote`).

unless the expression ends in `| quote` or sits inside a quoted word: a quote
that opens at the start of a word and closes at its end, so an apostrophe inside
a word (`don't`) quotes nothing. A literal target such as `2>/dev/null` is not a
path expression and is not refused.

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
PATHLIKE = re.compile(r"\b\w+_(path|dir|file|root|prefix)\b|\b\w*dir\b|\.path\b|\brole_path\b|~\s*['\"]/")
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


WORD_END = " \t\n;|&)"


def in_quoted_word(s, start, end):
    """Whether s[start:end] sits inside a quoted word: walking the string as a
    shell would (skipping the insides of other {{ ... }}), the quote open at
    `start` began at the start of a word and closes at the end of one. An
    apostrophe inside a word (`don't`) opens no quoted word."""
    i, quote, opened = 0, None, -1
    while i < start:
        if s.startswith("{{", i):
            j = s.find("}}", i)
            i = len(s) if j < 0 else j + 2
            continue
        c = s[i]
        if quote:
            if c == quote:
                quote = None
        elif c in "'\"":
            quote, opened = c, i
        i += 1
    if not quote or (opened > 0 and s[opened - 1] not in " \t\n=("):
        return False
    i = end
    while i < len(s):
        if s.startswith("{{", i):
            j = s.find("}}", i)
            i = len(s) if j < 0 else j + 2
            continue
        if s[i] == quote:
            return i + 1 >= len(s) or s[i + 1] in WORD_END
        i += 1
    return False


def command_hits(s):
    return [m.group(1).strip() for m in EXPR.finditer(s)
            if PATHLIKE.search(m.group(1)) and not QUOTED.search(m.group(1).strip())
            and not in_quoted_word(s, m.start(), m.end())]


def shell_hits(s):
    # In the shell any unquoted path splits: an argument, or a redirect target
    # after any operator (>, >>, <, 2>, &>). A literal target such as 2>/dev/null
    # holds no expression, so only the target decides.
    return command_hits(s)


# The modules whose string is split, and the rule each gets.
MODULES = (
    (("ansible.builtin.command", "ansible.legacy.command", "command"), command_hits, "command"),
    (("ansible.builtin.script", "ansible.legacy.script", "script"), command_hits, "script"),
    (("ansible.builtin.shell", "ansible.legacy.shell", "shell"), shell_hits, "shell"),
    (("ansible.builtin.raw", "ansible.legacy.raw", "raw"), shell_hits, "raw"),
)


def sites(root):
    """{"<file> :: <task name>": [expressions]} for every site found."""
    found = {}
    files = sorted(root.glob("ops/ansible/roles/*/tasks/**/*.yml")) + sorted(root.glob("ops/ansible/roles/*/handlers/**/*.yml"))
    for f in files:
        doc = yaml.load(f.read_text(), Loader=Loader)
        rel = f.relative_to(root).as_posix()
        for t in tasks(doc or []):
            for mods, rule, kind in MODULES:
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
                    f"or | quote in shell: and raw:")
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
            "- name: not a redirect\n  ansible.builtin.shell: \"echo hello 2>/dev/null\"\n")
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
    # The wider shapes: any unquoted path in shell:, a redirect onto a log path
    # after 2> or &>, the wider names, ansible.legacy.*, raw and script, handlers
    # in subfolders, and an apostrophe that does not open a quoted word.
    with tempfile.TemporaryDirectory() as d:
        r = Path(d)
        tdir = r / "ops/ansible/roles/wide/tasks"
        hdir = r / "ops/ansible/roles/wide/handlers/sub"
        tdir.mkdir(parents=True)
        hdir.mkdir(parents=True)
        (r / "ops/ci").mkdir(parents=True)
        (tdir / "main.yml").write_text(
            "- name: shell argument\n  ansible.builtin.shell: \"cat {{ wide_secret_path }} | wc -c\"\n"
            "- name: stderr redirect\n  ansible.builtin.shell: \"run 2> {{ wide_log_path }}\"\n"
            "- name: both redirect\n  ansible.builtin.shell: \"run &> {{ wide_log_path }}\"\n"
            "- name: devnull\n  ansible.builtin.shell: \"run 2>/dev/null\"\n"
            "- name: file name\n  ansible.builtin.command: \"docker compose -f {{ wide_compose_file }} up -d\"\n"
            "- name: root name\n  ansible.builtin.command: \"ls {{ wide_data_root }}\"\n"
            "- name: prefix name\n  ansible.builtin.command: \"ls {{ wide_prefix }}\"\n"
            "- name: bindir name\n  ansible.builtin.command: \"{{ wide_bindir }}/tool\"\n"
            "- name: workdir name\n  ansible.builtin.command: \"make -C {{ workdir }}\"\n"
            "- name: item path\n  ansible.builtin.command: \"rm {{ item.path }}\"\n"
            "- name: legacy command\n  ansible.legacy.command: \"cat {{ wide_key_path }}\"\n"
            "- name: legacy shell\n  ansible.legacy.shell: \"cat {{ wide_key_path }}\"\n"
            "- name: raw\n  ansible.builtin.raw: \"cat {{ wide_key_path }}\"\n"
            "- name: script\n  ansible.builtin.script: \"files/x.sh {{ wide_key_path }}\"\n"
            "- name: apostrophe\n  ansible.builtin.shell: \"echo don't {{ wide_key_path }} won't\"\n"
            "- name: quoted word still passes\n  ansible.builtin.shell: \"cat '{{ wide_key_path }}'\"\n"
            "- name: quoted with a filter inside\n  ansible.builtin.shell: \"cat \\\"{{ wide_key_path | default('') }}\\\"\"\n"
            "- name: not a path name\n  ansible.builtin.command: \"echo {{ wide_profile }}\"\n")
        (hdir / "main.yml").write_text("- name: nested handler\n  ansible.builtin.command: \"cat {{ wide_key_path }}\"\n")
        rc, out = run(r)
        assert rc == 1, out
        for name in ("shell argument", "stderr redirect", "both redirect", "file name", "root name", "prefix name",
                     "bindir name", "workdir name", "item path", "legacy command", "legacy shell", "raw", "script",
                     "apostrophe"):
            assert f"wide/tasks/main.yml :: {name}: templates" in out, (name, out)
        assert "wide/handlers/sub/main.yml :: nested handler: templates" in out, out
        for name in ("devnull", "quoted word still passes", "quoted with a filter inside", "not a path name"):
            assert f":: {name}:" not in out, (name, out)
    print("self-test ok: a split command, script, shell or raw path is refused, wherever it sits and whatever the "
          "redirect operator, with the wider names and in nested handlers; argv, | quote, a quoted word and a "
          "non-path pass, and an apostrophe inside a word quotes nothing; the baseline lets a known site pass, "
          "refuses a new one and a stale line")
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
