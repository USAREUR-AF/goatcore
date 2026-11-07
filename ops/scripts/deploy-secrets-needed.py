#!/usr/bin/env python3
"""Which environment variables a component's deploy templates for a goatnet, and which are missing here.

A deploy reads secrets from the deployer's environment: group_vars/<group>/vars.yml binds
`<component>_…: "{{ lookup('env', 'NAME') }}"` and the play or role templates that variable.
A fresh clone has none of them, and until 2026-10-27 the first anyone heard of it was an
assert on the host, one variable per run. This derives the full set up front — the env
lookups in the member groups' vars, intersected with the variable names the component's
play and role actually reference — so the driver can refuse before it touches a host, naming
every variable at once.

usage: deploy-secrets-needed.py --repo DIR --component NAME [--ledger NAME] --play PATH GROUP...
       prints one line per needed variable: `<ansible var> <ENV NAME> present|MISSING`
       exit 0 all present · 1 some missing · 2 cannot derive
"""
import argparse, os, re, sys

LOOKUP = re.compile(r"""^([a-z][a-z0-9_]*):\s.*lookup\(\s*['"]env['"]\s*,\s*['"]([A-Z][A-Z0-9_]*)['"]""")
WORD = re.compile(r"\b[a-z][a-z0-9_]+\b")


def env_bindings(repo, group):
    """ansible var -> ENV NAME, for every env lookup in the group's vars files."""
    out = {}
    d = os.path.join(repo, "ops/ansible/group_vars", group)
    if not os.path.isdir(d):
        return out
    for fn in sorted(os.listdir(d)):
        if not fn.endswith((".yml", ".yaml")):
            continue
        try:
            with open(os.path.join(d, fn), encoding="utf-8", errors="replace") as fh:
                for line in fh:
                    m = LOOKUP.match(line)
                    if m:
                        out[m.group(1)] = m.group(2)
        except OSError:
            continue
    return out


def referenced_words(repo, play, names):
    """Every lowercase identifier in the play and in roles/<name>/ for each candidate name."""
    words = set()
    paths = [play]
    for n in names:
        role = os.path.join(repo, "ops/ansible/roles", n)
        for root, _d, files in os.walk(role):
            paths += [os.path.join(root, f) for f in files]
    for p in paths:
        try:
            with open(p, encoding="utf-8", errors="replace") as fh:
                words |= set(WORD.findall(fh.read()))
        except OSError:
            continue
    # One hop through an import_playbook alias: the imported play's words count too.
    try:
        with open(play, encoding="utf-8", errors="replace") as fh:
            lines = fh.readlines()
        for line in lines:
            i = line.find("import_playbook:")
            if i >= 0:
                imp = os.path.join(os.path.dirname(play), line[i + len("import_playbook:"):].strip())
                with open(imp, encoding="utf-8", errors="replace") as fh:
                    words |= set(WORD.findall(fh.read()))
    except OSError:
        return words
    return words


# Environment names that are the shell's own, not secrets: a binding that reads one
# (or falls back to one) is a path or a user name, never a need.
NOT_SECRETS = {"HOME", "USER", "LOGNAME", "PATH", "PWD", "SHELL", "TMPDIR"}
ENV_ARG = re.compile(r"""lookup\(\s*['"]env['"]\s*,\s*""")
ROLE_BINDING = re.compile(r"""^([a-z][a-z0-9_]*):\s*["'](.*)["']\s*(#.*)?$""")
# A role that accepts another variable in its place says so on the binding's line:
#   portal_operator_api_key: "{{ lookup('env', 'PORTAL_OPERATOR_API_KEY') }}"  # or: PORTAL_OPERATOR_API_KEYS
OR_NAMES = re.compile(r"#\s*or:\s*([A-Z][A-Z0-9_]*(?:\s*,\s*[A-Z][A-Z0-9_]*)*)")
# A binding read only in one configuration says which, on the same line:
#   portal_db_password: "{{ lookup('env', portal_db_password_env) }}"  # when: portal_store=postgres
# It is needed when the goatnet's value (else the role default) is that value.
WHEN = re.compile(r"#.*?\bwhen:\s*([a-z][a-z0-9_]*)\s*=\s*([A-Za-z0-9_.-]+)")
SCALAR = re.compile(r"^([a-z][a-z0-9_]*):\s*(\S.*)$")


def site_vars(repo, group):
    """The scalar top-level values in the group's vars files (site_id and the like)."""
    out = {}
    d = os.path.join(repo, "ops/ansible/group_vars", group)
    if not os.path.isdir(d):
        return out
    for fn in sorted(os.listdir(d)):
        if fn.endswith((".yml", ".yaml")):
            try:
                with open(os.path.join(d, fn), encoding="utf-8", errors="replace") as fh:
                    for line in fh:
                        m = re.match(r"^([a-z][a-z0-9_]*):\s*(\S.*)$", line)
                        if m:
                            out.setdefault(m.group(1), m.group(2).split(" #")[0].strip().strip("'\""))
            except OSError:
                continue
    return out


def _env_arg(expr, i, svars, rvals=None):
    """The environment name a lookup('env', …) argument starting at expr[i] names, or None
    when it is computed from something this scan cannot resolve. Literal names, the one
    computed shape the roles use: (site_id … | upper [| replace('-','_')]) +/~ '_SUFFIX',
    and a variable (group value first, then the role's own) holding a literal name or
    "{{ site_id … }}_SUFFIX"."""
    rest = expr[i:]
    m = re.match(r"""['"]([A-Z][A-Z0-9_]*)['"]""", rest)
    if m:
        return m.group(1)
    m = re.match(r"""([a-z][a-z0-9_]*)\s*\)""", rest)
    if m:
        val = svars.get(m.group(1)) or (rvals or {}).get(m.group(1)) or ""
        if re.fullmatch(r"[A-Z][A-Z0-9_]*", val):
            return val
        t = re.fullmatch(r"""\{\{\s*(site_id_upper|site_id)\b([^}]*)\}\}([A-Z0-9_]+)""", val)
        return _site_name(t.group(1), t.group(2), t.group(3), svars) if t else None
    m = re.match(r"""\(\s*(site_id_upper|site_id)\b(.*?)\)\s*[+~]\s*['"]([A-Z0-9_]+)['"]""", rest)
    if not m:
        return None
    return _site_name(m.group(1), m.group(2), m.group(3), svars)


def _site_name(base, filters, suffix, svars):
    """site_id (or site_id_upper) through the filters, then the suffix; None when unknown."""
    if base == "site_id_upper":
        val = svars.get("site_id_upper")
        if not val:
            d = re.search(r"""default\(\s*['"]([A-Z0-9_]+)['"]""", filters)
            val = d.group(1) if d else None
        return (val + suffix) if val and "{" not in val else None
    sid = svars.get("site_id")
    if not sid or "{" in sid:
        return None
    sid = sid.upper()
    if "replace" in filters:
        sid = sid.replace("-", "_")
    return sid + suffix


def binding_alternatives(expr, svars, rvals=None):
    """For a role-default value: the environment names any one of which satisfies it, or
    None when it is not a need — it has a literal or shell-path fallback (`| default('x')`,
    `| default('', true)`), reads only shell names, or computes a name this scan cannot
    resolve."""
    if re.search(r"""default\(\s*['"]""", expr):
        return None   # a literal fallback (an empty string included): optional by declaration
    names = []
    for m in ENV_ARG.finditer(expr):
        n = _env_arg(expr, m.end(), svars, rvals)
        if n is None:
            return None
        names.append(n)
    if not names or any(n in NOT_SECRETS for n in names):
        return None
    return names


def role_default_bindings(repo, roles, group_vars_names, svars, play_words):
    """ansible var -> [ENV NAME, …] (any one suffices) for the env lookups in the
    defaults and vars of `roles` that no member group overrides and that the play or
    the role's own tasks and templates reference. Role defaults are where a release
    token like goat-get's GOAT_CLI_RELEASE_TOKEN lives; group_vars never name it."""
    out = {}
    for r in roles:
        rdir = os.path.join(repo, "ops/ansible/roles", r)
        used = set(play_words)
        for root, _d, files in os.walk(rdir):
            if os.path.basename(root) in ("defaults", "vars"):
                continue
            for f in files:
                try:
                    with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
                        used |= set(WORD.findall(fh.read()))
                except OSError:
                    continue
        role_lines = []
        for sub in ("defaults", "vars"):
            d = os.path.join(rdir, sub)
            if not os.path.isdir(d):
                continue
            for fn in sorted(os.listdir(d)):
                if not fn.endswith((".yml", ".yaml")):
                    continue
                try:
                    with open(os.path.join(d, fn), encoding="utf-8", errors="replace") as fh:
                        role_lines += fh.read().splitlines()
                except OSError:
                    continue
        rvals = {}
        for line in role_lines:
            sm = SCALAR.match(line)
            if sm:
                rvals.setdefault(sm.group(1), sm.group(2).split(" #")[0].strip().strip("'\""))
        for line in role_lines:
            m = ROLE_BINDING.match(line)
            if not m or "lookup(" not in m.group(2):
                continue
            var = m.group(1)
            if var in group_vars_names or var not in used:
                continue
            w = WHEN.search(m.group(3) or "")
            if w and (svars.get(w.group(1)) or rvals.get(w.group(1))) != w.group(2):
                continue
            alts = binding_alternatives(m.group(2), svars, rvals)
            if alts:
                o = OR_NAMES.search(m.group(3) or "")
                if o:
                    alts += [n.strip() for n in o.group(1).split(",") if n.strip() not in alts]
                out[var] = alts
    return out


def posture_available(repo):
    """Names with a value or reference under the declared secrets posture; the
    plain environment when the posture module cannot be read."""
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location("goat_posture", os.path.join(repo, "ops/secrets/posture.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod.available(repo)
    except Exception:
        return {k for k, v in os.environ.items() if v}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--component", required=True)
    ap.add_argument("--ledger")
    ap.add_argument("--play", required=True)
    ap.add_argument("groups", nargs="+")
    a = ap.parse_args()
    if not os.path.isfile(a.play):
        print(f"deploy-secrets-needed: play not found: {a.play}", file=sys.stderr)
        return 2
    names = [a.component] + ([a.ledger] if a.ledger and a.ledger != a.component else [])
    needed = needed_for(a.repo, a.play, names, a.groups)
    # Presence is the declared posture's (ADR 1204): under an env-file, git-crypt or
    # sops store the values are in that file, not yet in this shell.
    have = posture_available(a.repo)
    missing = 0
    for var, alts in sorted(needed.items()):
        present = any(e in have for e in alts)
        missing += not present
        print(f"{var} {'|'.join(alts)} {'present' if present else 'MISSING'}")
    return 1 if missing else 0


def play_words(play):
    try:
        with open(play, encoding="utf-8", errors="replace") as fh:
            text = fh.read()
    except OSError:
        return set()
    return set(WORD.findall(text))


INCLUDED_ROLE = re.compile(r"""(?:include_role|import_role):\s*\n\s+name:\s*['"]?([A-Za-z0-9_.-]+)['"]?\s*$""", re.M)


def role_closure(repo, roles):
    """`roles` and every role they pull in: include_role / import_role by a literal
    name in their tasks and handlers, and meta dependencies. A store role the
    component includes (goat-zenoh-portal -> goat-zenoh-portal-store) holds its
    database password binding, so it is part of what the deploy needs."""
    seen, queue = [], list(roles)
    while queue:
        r = queue.pop(0)
        if r in seen:
            continue
        seen.append(r)
        rdir = os.path.join(repo, "ops/ansible/roles", r)
        if not os.path.isdir(rdir):
            continue
        for sub in ("tasks", "handlers"):
            d = os.path.join(rdir, sub)
            for root, _d, files in os.walk(d):
                for f in sorted(files):
                    if not f.endswith((".yml", ".yaml")):
                        continue
                    try:
                        with open(os.path.join(root, f), encoding="utf-8", errors="replace") as fh:
                            queue += INCLUDED_ROLE.findall(fh.read())
                    except OSError:
                        continue
        meta = os.path.join(rdir, "meta", "main.yml")
        if os.path.isfile(meta):
            try:
                import yaml
                with open(meta, encoding="utf-8") as stream:
                    deps = (yaml.safe_load(stream) or {}).get("dependencies") or []
            except Exception:
                deps = []
            for dep in deps:
                name = dep.get("role") if isinstance(dep, dict) else dep
                if isinstance(name, str) and "{" not in name:
                    queue.append(name)
    return seen


def needed_for(repo, play, names, groups):
    """ansible var -> [ENV NAME, …]: the group_vars env bindings the play and roles
    reference (one name each), then the role-default bindings no group overrides
    (any one of the names suffices, e.g. GOAT_CLI_RELEASE_TOKEN|GH_TOKEN|GHCR_READ_TOKEN)."""
    words = referenced_words(repo, play, names)
    needed = {}
    gv_names = set()
    svars = {}
    for g in groups:
        sv = site_vars(repo, g)
        gv_names |= set(sv)
        for k, v in sv.items():
            svars.setdefault(k, v)
        for var, env in env_bindings(repo, g).items():
            if var in words:
                needed[var] = [env]
    for var, alts in role_default_bindings(repo, role_closure(repo, names), gv_names, svars, play_words(play)).items():
        needed.setdefault(var, alts)
    # An outbound channel whose empty value turns it off (ops/secrets/off-when-empty.txt)
    # is never a need: a goatnet with no PagerDuty, Slack or mail account still deploys.
    off = off_when_empty(repo)
    return {var: alts for var, alts in needed.items() if not all(a in off for a in alts)}


def off_when_empty(repo):
    """The names in ops/secrets/off-when-empty.txt: first word of each non-comment line."""
    out = set()
    try:
        with open(os.path.join(repo, "ops/secrets/off-when-empty.txt"), encoding="utf-8") as fh:
            for line in fh:
                word = line.split("#", 1)[0].split()
                if word:
                    out.add(word[0])
    except OSError:
        return out   # no list in this checkout: every binding stays a need, as before
    return out


if __name__ == "__main__":
    sys.exit(main())
