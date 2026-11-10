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
    m = re.match(r"""['"]([A-Z][A-Z0-9_]*)['"]\s*\)""", rest)
    if m:
        return m.group(1)
    m = re.match(r"""([a-z][a-z0-9_]*)\s*\)""", rest)
    if m:
        return _name_in(m.group(1), svars, rvals or {})
    m = re.match(r"""\(\s*(site_id_upper|site_id)\b(.*?)\)\s*[+~]\s*['"]([A-Z0-9_]+)['"]""", rest)
    if not m:
        return None
    return _site_name(m.group(1), m.group(2), m.group(3), svars)


def _name_in(var, svars, rvals, depth=0):
    """The environment name variable `var` holds: the group's value first, then the
    roles' own; a literal name, "{{ site_id … }}_SUFFIX", or one more variable
    "{{ other_var | default('NAME') }}" (audit-subscriber's root-credential names
    default to the minio role's `minio_root_user_envvar`). None when unresolvable."""
    val = svars.get(var) or rvals.get(var) or ""
    if re.fullmatch(r"[A-Z][A-Z0-9_]*", val):
        return val
    t = re.fullmatch(r"""\{\{\s*(site_id_upper|site_id)\b([^}]*)\}\}([A-Z0-9_]+)""", val)
    if t:
        return _site_name(t.group(1), t.group(2), t.group(3), svars)
    t = re.fullmatch(r"""\{\{\s*([a-z][a-z0-9_]*)\s*(?:\|\s*default\(\s*['"]([A-Z0-9_]*)['"]\s*(?:,\s*true\s*)?\)\s*)?\}\}""", val)
    if t and depth < 3:
        got = _name_in(t.group(1), svars, rvals, depth + 1)
        if got:
            return got
        return t.group(2) or None
    return None


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


# --- env lookups in the tasks a deploy runs ---------------------------------------
# A role can read a secret in a task, not in its defaults: audit-subscriber resolves
# its S3 keys and the MinIO root pair with set_fact, then asserts them, the root pair
# in an included lifecycle.yml that runs after the container is recreated. A check
# that read only defaults let that deploy replace the container and stop half-way.
# So the tasks the play runs are walked too: its own pre_tasks/tasks/post_tasks, each
# role (meta dependencies first, then tasks/main.yml), include_tasks/import_tasks and
# include_role/import_role by a literal name, with the `when:` conditions of every
# enclosing block, include and role entry carried down.

TASK_NOT_NEEDS = {"name", "when", "changed_when", "failed_when", "until", "msg", "fail_msg",
                  "success_msg", "tags", "notify", "register", "loop", "with_items",
                  "with_dict", "loop_control", "no_log", "become", "delegate_to", "listen"}
INCLUDE_TASKS = ("include_tasks", "import_tasks", "ansible.builtin.include_tasks", "ansible.builtin.import_tasks")
INCLUDE_ROLE = ("include_role", "import_role", "ansible.builtin.include_role", "ansible.builtin.import_role")
TRUE_WORDS = {"true", "yes", "on", "1"}
FALSE_WORDS = {"false", "no", "off", "0", ""}


def _load_yaml(path):
    try:
        import yaml
        with open(path, encoding="utf-8", errors="replace") as fh:
            return yaml.safe_load(fh)
    except Exception:
        return None


def _value(var, svars, rvals):
    """A variable's scalar value (group first, then the roles'), or None when unknown or templated."""
    v = svars.get(var, rvals.get(var))
    if v is None or "{" in str(v):
        return None
    return str(v).strip().strip("'\"")


def eval_when(cond, svars, rvals):
    """True / False for the condition shapes the roles use on a variable; None when
    this scan cannot decide it (the task then counts: a deploy may well reach it)."""
    if isinstance(cond, bool):
        return cond
    c = str(cond).strip()
    while c.startswith("(") and c.endswith(")") and c.count("(") == 1:
        c = c[1:-1].strip()
    if " or " in c:
        return None
    if " and " in c:
        parts = [eval_when(x, svars, rvals) for x in c.split(" and ")]
        if False in parts:
            return False
        return None if None in parts else True
    if c.startswith("not "):
        r = eval_when(c[4:], svars, rvals)
        return None if r is None else not r
    m = re.fullmatch(r"""\(?\s*([a-z_][a-z0-9_]*)\s*(?:\|\s*default\([^)]*\)\s*)?(\|\s*(bool|length|trim)\s*)?\)?\s*(?:(==|!=|>=|>|<=|<)\s*(\S+))?""", c)
    if not m:
        return None
    val = _value(m.group(1), svars, rvals)
    if val is None:
        return None
    filt, op, rhs = m.group(3), m.group(4), m.group(5)
    if filt == "length":
        left = len(val)
        if op is None:
            return left > 0
        try:
            n = int(rhs)
        except ValueError:
            return None
        return {"==": left == n, "!=": left != n, ">": left > n, ">=": left >= n, "<": left < n, "<=": left <= n}[op]
    if op in ("==", "!="):
        eq = val == rhs.strip("'\"")
        return eq if op == "==" else not eq
    if op:
        return None
    low = val.lower()
    if low in TRUE_WORDS:
        return True
    if low in FALSE_WORDS:
        return False
    return True if filt != "bool" else None


def _conds(task):
    w = task.get("when") if isinstance(task, dict) else None
    if w is None:
        return []
    return list(w) if isinstance(w, list) else [w]


def _strings(node, key=None):
    """(key, string) for every string leaf, skipping keys that never make a need."""
    if isinstance(node, str):
        yield key, node
    elif isinstance(node, dict):
        for k, v in node.items():
            if k not in TASK_NOT_NEEDS:
                yield from _strings(v, k if isinstance(k, str) else key)
    elif isinstance(node, list):
        for v in node:
            yield from _strings(v, key)


class _TaskWalk:
    def __init__(self, repo, svars, rvals):
        self.repo, self.svars, self.rvals = repo, svars, rvals
        self.out, self.seen, self.stack = {}, set(), []

    def live(self, conds):
        """False when a carried condition is decidedly false or the guard reads the env itself."""
        for c in conds:
            if "lookup(" in str(c):
                return False
            if eval_when(c, self.svars, self.rvals) is False:
                return False
        return True

    def tasks(self, items, base_dir, role, conds):
        for t in items or []:
            if not isinstance(t, dict):
                continue
            here = conds + _conds(t)
            if not self.live(here) or "never" in (t.get("tags") or []):
                continue
            if any(k in t for k in ("block", "rescue", "always")):
                for k in ("block", "rescue", "always"):
                    self.tasks(t.get(k), base_dir, role, here)
                continue
            inc = next((t[k] for k in INCLUDE_TASKS if k in t), None)
            if inc is not None:
                f = inc.get("file") if isinstance(inc, dict) else inc
                if isinstance(f, str) and "{" not in f:
                    self.file(os.path.join(base_dir, f), base_dir, role, here)
                continue   # an include's vars: are parameters; the included file decides
            rinc = next((t[k] for k in INCLUDE_ROLE if k in t), None)
            if rinc is not None:
                if isinstance(rinc, dict) and isinstance(rinc.get("name"), str) and "{" not in rinc["name"]:
                    self.role(rinc["name"], here, rinc.get("tasks_from") or "main")
                continue
            self.task(t, role)

    def task(self, t, role):
        facts = {}
        for mod in ("set_fact", "ansible.builtin.set_fact"):
            if isinstance(t.get(mod), dict):
                facts.update(t[mod])
        if isinstance(t.get("vars"), dict):
            facts.update(t["vars"])
        for key, text in _strings(t):
            if "lookup(" not in text or re.search(r"\bif\b.*\belse\b", text, re.S):
                continue
            alts = binding_alternatives(text, self.svars, self.rvals)
            if not alts:
                continue
            var = key if key in facts and isinstance(key, str) and re.fullmatch(r"[a-z][a-z0-9_]*", key) else f"{role}:{alts[0]}"
            self.out.setdefault(var, alts)

    def file(self, path, base_dir, role, conds):
        sig = (os.path.normpath(path), tuple(map(str, conds)))
        if sig in self.seen or len(self.seen) > 500:
            return
        self.seen.add(sig)
        self.tasks(_load_yaml(path), base_dir, role, conds)

    def role(self, name, conds, entry="main"):
        rdir = os.path.join(self.repo, "ops/ansible/roles", name)
        if not os.path.isdir(rdir) or name in self.stack:
            return
        self.stack.append(name)
        try:
            self._role(rdir, name, conds, entry)
        finally:
            self.stack.pop()

    def _role(self, rdir, name, conds, entry):
        meta = _load_yaml(os.path.join(rdir, "meta", "main.yml")) or {}
        for dep in (meta.get("dependencies") or []) if isinstance(meta, dict) else []:
            dname = dep.get("role") if isinstance(dep, dict) else dep
            if isinstance(dname, str) and "{" not in dname:
                self.role(dname, conds + _conds(dep))
        tdir = os.path.join(rdir, "tasks")
        for ext in (".yml", ".yaml"):
            if os.path.isfile(os.path.join(tdir, entry + ext)):
                self.file(os.path.join(tdir, entry + ext), tdir, name, conds)
                break

    def play(self, path):
        doc = _load_yaml(path)
        base = os.path.dirname(path)
        for p in doc if isinstance(doc, list) else []:
            if not isinstance(p, dict):
                continue
            if "import_playbook" in p:
                imp = os.path.join(base, str(p["import_playbook"]))
                if os.path.isfile(imp) and imp != path:
                    self.play(imp)
                continue
            pc = _conds(p)
            self.tasks(p.get("pre_tasks"), base, "play", pc)
            for r in p.get("roles") or []:
                rname = r.get("role") or r.get("name") if isinstance(r, dict) else r
                if isinstance(rname, str) and "{" not in rname:
                    self.role(rname, pc + (_conds(r) if isinstance(r, dict) else []))
            self.tasks(p.get("tasks"), base, "play", pc)
            self.tasks(p.get("post_tasks"), base, "play", pc)


def play_roles(play):
    """The literal role names a play's `roles:` lists (through import_playbook)."""
    out, doc = [], _load_yaml(play)
    for p in doc if isinstance(doc, list) else []:
        if not isinstance(p, dict):
            continue
        if "import_playbook" in p:
            imp = os.path.join(os.path.dirname(play), str(p["import_playbook"]))
            if os.path.isfile(imp) and imp != play:
                out += play_roles(imp)
        for r in p.get("roles") or []:
            n = r.get("role") or r.get("name") if isinstance(r, dict) else r
            if isinstance(n, str) and "{" not in n:
                out.append(n)
    return out


def role_scalars(repo, roles):
    """The scalar top-level values of the roles' defaults and vars, first role first."""
    out = {}
    for r in roles:
        for sub in ("defaults", "vars"):
            d = os.path.join(repo, "ops/ansible/roles", r, sub)
            if not os.path.isdir(d):
                continue
            for fn in sorted(os.listdir(d)):
                if not fn.endswith((".yml", ".yaml")):
                    continue
                try:
                    with open(os.path.join(d, fn), encoding="utf-8", errors="replace") as fh:
                        for line in fh:
                            m = SCALAR.match(line)
                            if m:
                                out.setdefault(m.group(1), m.group(2).split(" #")[0].strip().strip("'\""))
                except OSError:
                    continue
    return out


def task_bindings(repo, play, roles, svars):
    """ansible var (or `<role>:<NAME>` for a lookup no variable holds) -> [ENV NAME, …],
    for the env lookups in the tasks the play runs on this goatnet."""
    w = _TaskWalk(repo, svars, role_scalars(repo, roles))
    w.play(play)
    return w.out


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
    roles = role_closure(repo, names + [r for r in play_roles(play) if r not in names])
    for var, alts in role_default_bindings(repo, roles, gv_names, svars, play_words(play)).items():
        needed.setdefault(var, alts)
    # The lookups the tasks make themselves (set_fact, an assert, a copy's content),
    # the asserts of a later phase included: a name already found above stays as found.
    found = {e for alts in needed.values() for e in alts}
    for var, alts in task_bindings(repo, play, roles, svars).items():
        if var not in needed and not set(alts) & found:
            needed[var] = alts
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
