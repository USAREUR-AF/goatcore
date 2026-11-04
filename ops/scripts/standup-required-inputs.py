#!/usr/bin/env python3
"""standup-required-inputs — every input a site's standup will demand, checked
before any host is touched, all at once.

A standup run that stops on a missing variable has learned one fact for the
price of a run; the next run learns the next one. This scan reads what the
run will reach and names every gap in one pass:

  1. the plan: the playbooks each phase runs and the group each targets, read
     from `goat-standup.sh <site> --plan` (the same lines `goatbuilder standup
     --dry-run` prints — the parity gate keeps them identical);
  2. the role graph each playbook reaches (roles:, include/import_role,
     import_playbook, meta dependencies);
  3. every REQUIRED input in that graph: `x | mandatory`, and assert gates of
     the shapes `x is defined`, `x | length > 0`, `x != ''`;
  4. what the site supplies: group_vars/all + group_vars/<group>, the
     inventory's host and group vars, the play's own vars, role defaults that
     are not empty, extra vars the plan passes, and anything the reached
     plays and roles set at run time (register / set_fact);
  5. env-bound secrets: a supplied value of the form lookup('env', 'NAME')
     needs NAME available under the declared secrets posture — asked of
     ops/secrets/posture.py, the one place every caller asks (ADR 1204), so a
     name the posture can supply counts whether or not this shell holds it
     (skip with --no-secrets);
  6. leaf certificates the reached plays install from the controller (the
     router, the LAN mgmt identity, the names collector): each must be where
     ops/ansible/filter_plugins/goat_leaf.py looks, the site manifest's
     out_dir first (#2165), else a GB-071 row names the mint command. Leaves
     are operator key material, so --no-secrets skips them too;
  7. what an earlier step of the scanned range produces: a secret phase 3
     generates, a leaf phase 4 mints, a value a MANUAL step tells the operator to
     store. Each is reported as `produced by phase N of this run`, not as a gap,
     when that step is inside --from/--to and can do it, and when no step before
     it already reads it. A producer outside the range counts for nothing: by the
     phase-12 gate, what phase 3 should have written must be there. A name nothing in the plan produces stays a refusal. A joiner's secrets
     are never counted: they are its founder's, not a second genesis.

A requirement under a `when:` the scan can evaluate against the site's vars
is kept or dropped by it; one it cannot evaluate is reported as CONDITIONAL
(a warning, never a refusal), so the scan never claims more than it knows.

usage: standup-required-inputs.py <site> [--profile lan|internet]
                                  [--plan-file FILE] [--no-secrets] [--json] [--ansible-dir DIR]
exit 0 nothing missing · 1 inputs missing · 2 cannot derive
Offline: reads the repository; contacts no host.
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("standup-required-inputs: needs PyYAML (pip3 install pyyaml)")

REPO = Path(__file__).resolve().parents[2]
ANS = REPO / "ops" / "ansible"

MAGIC = {
    "item", "inventory_hostname", "inventory_hostname_short", "groups", "hostvars",
    "group_names", "play_hosts", "ansible_play_hosts", "omit", "playbook_dir",
    "role_path", "lookup", "true", "false", "none", "true", "target",
}
IDENT = r"[a-z_][a-z0-9_]*"
ENV_LOOKUP = re.compile(r"""lookup\(\s*['"]env['"]\s*,\s*['"]([A-Z][A-Z0-9_]*)['"]""")
WORD = re.compile(r"\b[a-z][a-z0-9_]+\b")
MANDATORY = re.compile(r"\b(" + IDENT + r")\s*\|\s*mandatory\b")
CONDITIONAL_LINE = re.compile(r"\s(?:and|or|if|else)\s|^\s*-?\s*when:|^\s*-\s*not\s")


def mandatory_vars(text):
    """`x | mandatory` outside a conditional expression: inside `a and (x | mandatory)` the
    filter is short-circuited away whenever `a` is false, so it is not an unconditional need."""
    out = []
    for line in text.splitlines():
        if CONDITIONAL_LINE.search(line):
            continue
        out += [m.group(1) for m in MANDATORY.finditer(line)]
    return out


# ---------------------------------------------------------------- the plan ---

def read_plan(site, profile, plan_file):
    if plan_file:
        text = Path(plan_file).read_text()
    else:
        cmd = [str(REPO / "ops/scripts/goat-standup.sh"), site, "--plan"]
        if profile:
            cmd += ["--profile", profile]
        try:
            text = subprocess.run(cmd, capture_output=True, text=True, timeout=120).stdout
        except (OSError, subprocess.TimeoutExpired) as e:
            sys.exit(f"standup-required-inputs: cannot read the plan ({e})")
    phases = []
    # A numbered phase, or a named observability leg that runs beside them
    # ('=== Observability leg "host" [A ∥] ...'; before the
    # renumbering it printed '=== Obs leg O1 ...'). A header this does not
    # match is a play the scan never reads, so both shapes stay matched.
    head = re.compile(r'^=== (?:Phase (\d+)|Observability leg "([^"]+)"|Obs leg (\S+)) \[([^\]]+)\] (.*?) ===$')
    cur = None
    for line in text.splitlines():
        m = head.match(line.strip())
        if m:
            leg = m.group(2) or m.group(3)
            cur = {"phase": m.group(1) or f"observability leg {leg}", "kind": m.group(4),
                   "label": m.group(5), "cmd": ""}
            phases.append(cur)
        elif cur is not None and line.startswith("    ") and not cur["cmd"]:
            cur["cmd"] = line.strip()
    return phases


GREP_GUARD = re.compile(r"""grep -qE '([^']+)' (\S+?) 2>/dev/null(?: &&|; then)""")
LS_GUARD = re.compile(r"\bls (\S+\.yml) >/dev/null 2>&1(?: &&|; then)")


def plays_in(cmd, default_group):
    """[(playbook, target group, extra vars)] for each ansible-preflight call in a command."""
    out = []
    for g in GREP_GUARD.finditer(cmd):
        f = ANS / g.group(2)
        try:
            pat = g.group(1).replace("[[:space:]]", r"\s")
            if not re.search(pat, f.read_text(), re.M):
                return []
        except OSError:
            return []
    # A phase guarded on a file the goatnet does not have reaches nothing. The
    # pubsub-state mirror is guarded this way (phase 12), so a goatnet with no
    # group_vars/<group>/pubsub-state.yml never runs it -- and a goatlite never
    # has one, since the mirror pushes state into a git server it has no part of.
    # Without this the scan called the play reached and the gate asked for two
    # secrets nothing would ever read, which a goatlite cannot supply at all.
    for g in LS_GUARD.finditer(cmd):
        if not (ANS / g.group(1)).exists():
            return []
    for seg in cmd.split("ansible-preflight.sh")[1:]:
        seg = re.split(r"\|\||&&|\)\s*;|;\s", seg)[0]
        toks = seg.split()
        play, extra, group, limit = None, {}, default_group, None
        i = 0
        while i < len(toks):
            t = toks[i]
            if t in ("-l", "--limit") and i + 1 < len(toks):
                limit = toks[i + 1].strip("'\"")
                i += 2   # -l narrows hosts inside the target group; the group's vars still apply
                continue
            if t == "-i":
                i += 2
                continue
            if t == "-e" and i + 1 < len(toks):
                kv = toks[i + 1].strip("'\"")
                if "=" in kv and not kv.startswith("@"):
                    k, v = kv.split("=", 1)
                    extra[k] = v
                    if k in ("target", "mgmt_host_target"):
                        group = v
                i += 2
                continue
            if t.endswith(".yml") and play is None:
                play = t.strip("'\"")
            i += 1
        if play and (ANS / play).is_file():
            out.append((play, group, extra, limit))
    return out


# ------------------------------------------------------------- yaml walking ---

_YAML = {}


def load_yaml(path):
    """Parsed once per scan, and read-only to every caller: the per-step walk reads each file again."""
    key = str(path)
    if key not in _YAML:
        try:
            _YAML[key] = yaml.safe_load(Path(path).read_text())
        except Exception:
            _YAML[key] = None
    return _YAML[key]


def task_lists(node):
    """Yield (task, inherited when-list) for every task, descending into blocks."""
    def walk(tasks, whens):
        if not isinstance(tasks, list):
            return
        for t in tasks:
            if not isinstance(t, dict):
                continue
            w = whens + as_list(t.get("when"))
            for key in ("block", "rescue", "always"):
                if key in t:
                    yield from walk(t[key], w)
            yield t, w
    yield from walk(node, [])


def as_list(v):
    if v is None:
        return []
    return v if isinstance(v, list) else [v]


class Graph:
    def __init__(self):
        self.roles = {}        # role -> list of when-chains it was reached under
        self.plays = []        # (play file, play dict, when-chain)
        self.set_at_runtime = set()
        self.params = set()   # vars handed to a role as params (roles: / include_role vars: / meta deps)

    def add_play_file(self, rel, whens=()):
        doc = load_yaml(ANS / rel)
        if not isinstance(doc, list):
            return
        for play in doc:
            if not isinstance(play, dict):
                continue
            if "import_playbook" in play:
                imp = str(play["import_playbook"])
                if "{{" not in imp:
                    self.add_play_file(str((Path(rel).parent / imp)), list(whens) + as_list(play.get("when")))
                continue
            w = list(whens) + as_list(play.get("when"))
            self.plays.append((rel, play, w))
            for r in as_list(play.get("roles")):
                name, rw = (r, []) if isinstance(r, str) else (r.get("role") or r.get("name"), as_list(r.get("when")))
                if isinstance(r, dict):
                    self.note_params(r)
                if name:
                    self.add_role(str(name), w + rw)
            for key in ("pre_tasks", "tasks", "post_tasks"):
                self.scan_tasks(play.get(key), w)

    def scan_tasks(self, tasks, whens):
        for t, tw in task_lists(tasks):
            for k in ("include_role", "import_role", "ansible.builtin.include_role", "ansible.builtin.import_role"):
                if k in t and isinstance(t[k], dict) and t[k].get("name"):
                    self.add_role(str(t[k]["name"]), list(whens) + tw)
                    self.note_params(t)
            self.note_runtime(t)

    def note_params(self, d):
        for k, v in d.items():
            if k == "vars" and isinstance(v, dict):
                self.params |= set(v)
            elif k not in ("role", "name", "when", "tags", "vars", "become", "include_role",
                           "import_role", "ansible.builtin.include_role", "ansible.builtin.import_role"):
                self.params.add(k)

    def note_runtime(self, t):
        if isinstance(t.get("register"), str):
            self.set_at_runtime.add(t["register"])
        for k in ("set_fact", "ansible.builtin.set_fact"):
            if isinstance(t.get(k), dict):
                self.set_at_runtime |= {x for x in t[k] if x != "cacheable"}
        if isinstance(t.get("vars"), dict):
            self.set_at_runtime |= set(t["vars"])

    def add_role(self, name, whens):
        if "{{" in name:
            return
        first = name not in self.roles
        self.roles.setdefault(name, []).append(list(whens))
        if not first:
            return
        rdir = ANS / "roles" / name
        meta = load_yaml(rdir / "meta" / "main.yml") or {}
        for d in as_list(meta.get("dependencies") if isinstance(meta, dict) else None):
            dn = d if isinstance(d, str) else (d.get("role") or d.get("name"))
            if isinstance(d, dict):
                self.note_params(d)
            if dn:
                self.add_role(str(dn), list(whens) + ([] if isinstance(d, str) else as_list(d.get("when"))))
        for f, fw in role_task_files(rdir):
            self.scan_tasks(load_yaml(f), list(whens) + fw)


# ------------------------------------------------------------ requirements ---

NONEMPTY = [
    re.compile(r"^\(?\s*(" + IDENT + r")\s*(?:\|\s*default\([^)]*\)\s*)?(?:\|\s*(?:string|trim|list)\s*)*\|\s*length\s*\)?\s*(?:>\s*0|>=\s*[1-9]\d*|!=\s*0)\s*$"),
    re.compile(r"^(" + IDENT + r")\s*!=\s*(?:''|\"\")\s*$"),
]
DEFINED = re.compile(r"^(" + IDENT + r")\s+is\s+defined\s*$")


def classify(expr):
    """[(var, 'defined'|'nonempty')] a single assert expression requires."""
    out = []
    for part in re.split(r"\s+and\s+", str(expr).strip()):
        part = part.strip()
        while part.startswith("(") and part.endswith(")") and part.count("(") == part.count(")") and _balanced(part[1:-1]):
            part = part[1:-1].strip()
        m = DEFINED.match(part)
        if m:
            out.append((m.group(1), "defined"))
            continue
        for rx in NONEMPTY:
            m = rx.match(part)
            if m:
                out.append((m.group(1), "nonempty"))
                break
    return out


def _balanced(s):
    d = 0
    for c in s:
        d += c == "("
        d -= c == ")"
        if d < 0:
            return False
    return d == 0


INCLUDE_KEYS = ("include_tasks", "import_tasks", "ansible.builtin.include_tasks", "ansible.builtin.import_tasks")


def role_task_files(rdir, entry="main.yml"):
    """[(task file, when-list it is reached under)] following include/import_tasks from main.yml."""
    out, seen = [], set()

    def visit(path, whens):
        if path in seen or not path.is_file():
            return
        seen.add(path)
        out.append((path, list(whens)))
        for t, tw in task_lists(load_yaml(path)):
            for k in INCLUDE_KEYS:
                v = t.get(k)
                f = v.get("file") if isinstance(v, dict) else v
                if isinstance(f, str) and "{{" not in f:
                    visit((path.parent / f).resolve(), list(whens) + tw)
    visit((rdir / "tasks" / entry).resolve(), [])
    return out


def requirements(graph):
    """[(var, need, where, when-chain)] over every reached play and role."""
    reqs = []

    def from_tasks(tasks, base_whens, where):
        if isinstance(base_whens, tuple) and len(base_whens) == 2 and isinstance(base_whens[1], list) and not (base_whens and base_whens[0] == "ANY"):
            base_whens, file_whens = base_whens
        else:
            file_whens = []
        for t, tw in task_lists(tasks):
            tw = list(file_whens) + tw
            for k in ("assert", "ansible.builtin.assert"):
                if k in t and isinstance(t[k], dict):
                    for e in as_list(t[k].get("that")):
                        for var, need in classify(e):
                            reqs.append((var, need, where, (base_whens, tw)))

    for rel, play, w in graph.plays:
        for key in ("pre_tasks", "tasks", "post_tasks"):
            from_tasks(play.get(key), w, rel)
        for v in mandatory_vars(Path(ANS / rel).read_text()):
            reqs.append((v, "defined", rel, (w, [])))
    for name, chains in graph.roles.items():
        rdir = ANS / "roles" / name
        w = ("ANY", chains)
        for f, fw in role_task_files(rdir):
            from_tasks(load_yaml(f), (w, fw), str(f.relative_to(ANS)))
        for f in sorted(rdir.rglob("*")):
            if f.is_file() and f.suffix in (".yml", ".j2", ".yaml") and "defaults" not in f.parts:
                try:
                    for v in mandatory_vars(f.read_text(errors="replace")):
                        reqs.append((v, "defined", str(f.relative_to(ANS)), (w, [])))
                except OSError:
                    pass   # an unreadable file in a role adds no requirement; the play run reports it
    return reqs


# ------------------------------------------------------------- what is set ---

def group_vars(group):
    vals = {}
    for g in ("all", group):
        d = ANS / "group_vars" / g
        files = sorted(d.glob("*.yml")) + sorted(d.glob("*.yaml")) if d.is_dir() else []
        single = ANS / "group_vars" / f"{g}.yml"
        if single.is_file():
            files.append(single)
        for f in files:
            doc = load_yaml(f)
            if isinstance(doc, dict):
                vals.update(doc)
    return vals


def inventory_vars(group):
    """Host and group vars the INI inventories (and host_vars/) give members of `group`."""
    keys = {}
    hosts = []
    for f in sorted((ANS / "inventory").glob("*.ini")):
        section = None
        for line in f.read_text(errors="replace").splitlines():
            s = line.strip()
            if not s or s.startswith(("#", ";")):
                continue
            if s.startswith("["):
                section = s.strip("[]")
                continue
            if section == group:
                hosts.append(s.split()[0])
                for kv in s.split()[1:]:
                    if "=" in kv:
                        k, v = kv.split("=", 1)
                        keys[k] = v
            elif section == f"{group}:vars" and "=" in s:
                k, v = s.split("=", 1)
                keys[k.strip()] = v.strip()
    for h in sorted(set(hosts)):
        for f in (ANS / "host_vars" / f"{h}.yml", ANS / "inventory" / "host_vars" / f"{h}.yml"):
            doc = load_yaml(f)
            if isinstance(doc, dict):
                for k, v in doc.items():
                    keys.setdefault(k, v)
    return keys


def group_hosts(group):
    hosts = set()
    for f in sorted((ANS / "inventory").glob("*.ini")):
        section = None
        for line in f.read_text(errors="replace").splitlines():
            st = line.strip()
            if not st or st.startswith(("#", ";")):
                continue
            if st.startswith("["):
                section = st.strip("[]")
                continue
            if section == group:
                hosts.add(st.split()[0])
    return hosts


def limit_matches(pattern, group):
    """Does an ansible --limit pattern select any host of the group? (globs and plain names)"""
    import fnmatch
    hosts = group_hosts(group)
    if not hosts:
        return True   # no inventory yet (a new site): assume the play will run
    return any(fnmatch.fnmatch(h, p) for p in pattern.split(":") for h in hosts | {group})


def role_defaults(graph):
    vals = {}
    for name in graph.roles:
        d = ANS / "roles" / name / "defaults"
        for f in sorted(d.glob("*.yml")) if d.is_dir() else []:
            doc = load_yaml(f)
            if isinstance(doc, dict):
                for k, v in doc.items():
                    vals.setdefault(k, v)
        v = load_yaml(ANS / "roles" / name / "vars" / "main.yml")
        if isinstance(v, dict):
            for k, val in v.items():
                vals[k] = val
    return vals


_POSTURE = None


def posture():
    """ops/secrets/posture.py as a module, or None (then only this shell's environment counts)."""
    global _POSTURE
    if _POSTURE is None:
        import importlib.util
        path = REPO / "ops" / "secrets" / "posture.py"
        try:
            spec = importlib.util.spec_from_file_location("goat_posture", path)
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            _POSTURE = mod
        except Exception:
            _POSTURE = False
    return _POSTURE or None


_AVAILABLE = None
_DSN = None


def _goat_leaf():
    import importlib.util
    spec = importlib.util.spec_from_file_location("goat_leaf", REPO / "ops" / "ansible" / "filter_plugins" / "goat_leaf.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def leaves_needed(g, known, site_id):
    """(leaf file in site/file form, required_by) for each controller-held leaf the reached plays install."""
    def plain(v, fallback):
        v = str(v) if v is not None else ""
        return v if v and "{{" not in v else fallback
    need = []
    if "goat-zenoh" in g.roles:
        site = plain(known.get("goat_router_leaf_site"), site_id)
        need += [(f"{site}/router-cert.pem", "roles/goat-zenoh"), (f"{site}/router-key.pem", "roles/goat-zenoh")]
    if any(rel.endswith("lan-traefik-routes.yml") for rel, _p, _w in g.plays):
        need += [(f"{site_id}/mgmt-identity-cert.pem", "lan-traefik-routes.yml"), (f"{site_id}/mgmt-identity-key.pem", "lan-traefik-routes.yml")]
    if "names-collector" in g.roles:
        for var in ("names_collector_cert_src", "names_collector_key_src"):
            v = plain(known.get(var), "")
            if v:
                need.append((v, "roles/names-collector"))
    return need


# ----------------------------------------------- produced by this same run ---

MANUAL_STORES = re.compile(r"(?:store it as|export)\s+([A-Z][A-Z0-9_]*)")
SAN_PLACEHOLDER = re.compile(r"TBD_[A-Z0-9_]*")


def phase3_can_write(store, environ=os.environ):
    """Whether newsite-secret-store.py would persist under this store, or why not. Mirrors its
    own refusals, by name only, so the gate does not promise a phase 3 that dies on arrival."""
    p = posture()
    if p is None:
        return False
    if store in p.VALUE_STORES or store == "openbao":
        return True
    if store == "1password":
        return bool(environ.get("GOAT_SECRET_VAULT"))
    if store == "custom":
        return bool(environ.get("GOAT_SECRET_INIT"))
    return False   # env persists nothing; any other store the generator does not write to


def phase3_names(site):
    """The names newsite-secrets.sh generates as the plan runs it: its SPEC, less the optional
    bootstrap pair that needs --bootstrap-operator, which the plan does not pass."""
    try:
        r = subprocess.run([str(REPO / "ops/scripts/newsite-secrets.sh"), site, "--spec-json"],
                           capture_output=True, text=True, timeout=60)
        spec = json.loads(r.stdout) if r.returncode == 0 else {}
    except (OSError, subprocess.TimeoutExpired, ValueError):
        return set()
    return {n for n, kind in spec.items() if not str(kind).startswith("pair:")}


def minted_leaves(site, cmd, known, repo=None):
    """The leaf files `mint-site-certs.sh <site>` will write, in site/file form, or an empty
    set when it would die instead: no manifest, a CA it cannot use, a SAN it cannot resolve."""
    repo = repo or REPO
    doc = load_yaml(repo / "ops/enrollment/manifests" / f"{site}-certs.yaml")
    if not isinstance(doc, dict) or doc.get("site_id") != site:
        return set()
    ca = doc.get("ca") or {}
    root = Path(os.path.expanduser(str(ca.get("root_cert", ""))))
    root = root if root.is_absolute() else repo / root
    have = [root.is_file(), Path(os.path.expanduser(str(ca.get("root_key", "")))).is_file()]
    if not all(have) and (any(have) or "--mint-ca-if-missing" not in cmd):
        return set()   # half a CA is refused; no CA is refused unless the plan mints one
    rows = [r for r in doc.get("leaves") or [] if isinstance(r, dict) and r.get("name")]
    for tbd in SAN_PLACEHOLDER.findall(json.dumps(rows)):
        var = "mesh_ip_a" if tbd.endswith(("_MESH_IP_A", "_CORE_A_MESH_IP")) else \
              "mesh_ip_b" if tbd.endswith(("_MESH_IP_B", "_CORE_B_MESH_IP")) else None
        if not var or empty(known.get(var)) or "{{" in str(known.get(var)):
            return set()
    return {f"{site}/{r['name']}-{part}.pem" for r in rows for part in ("cert", "key")}


class LaterSteps:
    """What the plan's own steps produce, each with its position in the plan. Order is the plan's
    order rather than the phase number, so an observability leg counts from where it starts."""

    def __init__(self, phases, site, store):
        self.position = {id(ph): i for i, ph in enumerate(phases)}
        self.secrets, self.mint = {}, None
        joiner = any("[joiner]" in ph["label"] for ph in phases)
        for i, ph in enumerate(phases):
            if ph["kind"] == "MANUAL":
                for name in MANUAL_STORES.findall(ph["cmd"]):
                    self.secrets.setdefault(name, (i, ph))
            elif "newsite-secrets.sh" in ph["cmd"] and not joiner and phase3_can_write(store):
                for name in phase3_names(site):
                    self.secrets.setdefault(name, (i, ph))
            elif "mint-site-certs.sh" in ph["cmd"] and self.mint is None:
                self.mint = (i, ph)


_WORDS = {}


def words_in(rel_or_role, files):
    """Every lower-case identifier in these files, cached: each play is read once per scan."""
    if rel_or_role not in _WORDS:
        words = set()
        for f in files:
            try:
                words |= set(WORD.findall(Path(f).read_text(errors="replace")))
            except OSError:
                pass   # an unreadable file templates nothing this scan can see
        _WORDS[rel_or_role] = words
    return _WORDS[rel_or_role]


def first_read(items, later, needs):
    """The plan position of the first step in `items` whose reached plays satisfy needs(graph,
    words), or None when none does."""
    best = None
    for ph, play, _extra in items:
        at = later.position[id(ph)]
        if best is not None and at >= best:
            continue
        g = Graph()
        g.add_play_file(play)
        words = set()
        for rel, _p, _w in g.plays:
            words |= words_in(rel, [ANS / rel])
        for name in g.roles:
            words |= words_in(f"role:{name}", [f for f in (ANS / "roles" / name).rglob("*") if f.is_file()])
        if needs(g, words):
            best = at
    return best


def deploy_secrets_needed():
    """ops/scripts/deploy-secrets-needed.py as a module (its role-default derivation), or None."""
    global _DSN
    if _DSN is None:
        import importlib.util
        try:
            spec = importlib.util.spec_from_file_location("deploy_secrets_needed", REPO / "ops" / "scripts" / "deploy-secrets-needed.py")
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            _DSN = mod
        except Exception:
            _DSN = False
    return _DSN or None


def env_supplied(name):
    global _AVAILABLE
    if os.environ.get(name):
        return True
    if _AVAILABLE is None:
        p = posture()
        try:
            _AVAILABLE = set(p.available(str(REPO))) if p else set()
        except Exception:
            _AVAILABLE = set()
    return name in _AVAILABLE


def empty(v):
    return v is None or (isinstance(v, (str, list, dict)) and len(v) == 0)


# --------------------------------------------------------- when-evaluation ---

class Unknown(Exception):
    pass


def evaluate(cond, vals):
    """True / False for a simple Ansible condition over raw site values; Unknown otherwise."""
    if isinstance(cond, bool):
        return cond
    s = str(cond).strip()
    if "{{" in s or "hostvars" in s or "groups[" in s or "ansible_" in s or ".stdout" in s or ".rc" in s:
        raise Unknown(s)
    py = s
    py = re.sub(r"(" + IDENT + r")\s+is\s+not\s+defined", lambda m: f"(not _d('{m.group(1)}'))", py)
    py = re.sub(r"(" + IDENT + r")\s+is\s+defined", lambda m: f"_d('{m.group(1)}')", py)
    py = re.sub(r"\|\s*default\(([^()]*)\)", r"@D(\1)", py)
    py = re.sub(r"\|\s*bool", "@B", py)
    py = re.sub(r"\|\s*length", "@L", py)
    py = re.sub(r"\|\s*(?:string|trim|lower)", "@S", py)
    if "|" in py:
        raise Unknown(s)

    def var(m):
        name = m.group(0)
        if name in ("and", "or", "not", "in", "true", "false", "True", "False", "none", "None", "_d"):
            return {"true": "True", "false": "False", "none": "None"}.get(name, name)
        return f"_v('{name}')"
    # Quote-aware identifier substitution.
    parts = re.split(r"('[^']*'|\"[^\"]*\")", py)
    for i in range(0, len(parts), 2):
        parts[i] = re.sub(IDENT, var, parts[i])
        parts[i] = re.sub(r"@D\(", ".d(", parts[i]).replace("@B", ".b()").replace("@L", ".l()").replace("@S", ".s()")
    py = "".join(parts)

    class V:
        def __init__(self, name, val, present):
            self.name, self.val, self.present = name, val, present

        def d(self, default=None, _true=False):
            return self if self.present and not (_true and empty(self.val)) else V(self.name, default, True)

        def b(self):
            self._need()
            v = self.val
            return v if isinstance(v, bool) else str(v).strip().lower() in ("true", "yes", "1", "on")

        def l(self):
            self._need()
            return len(self.val) if self.val is not None else 0

        def s(self):
            self._need()
            return str(self.val)

        def _need(self):
            if not self.present:
                raise Unknown(self.name)
            if isinstance(self.val, str) and "{{" in self.val:
                raise Unknown(self.name)

        def __eq__(self, o):
            self._need()
            return self.val == (o.val if isinstance(o, V) else o)

        def __ne__(self, o):
            return not self.__eq__(o)

        def __bool__(self):
            self._need()
            return bool(self.val)

        def __contains__(self, o):
            self._need()
            return o in self.val

        def __gt__(self, o):
            self._need()
            return self.val > o

        def __lt__(self, o):
            self._need()
            return self.val < o

        def __ge__(self, o):
            self._need()
            return self.val >= o

        def __le__(self, o):
            self._need()
            return self.val <= o

    def _v(name):
        return V(name, vals.get(name), name in vals)

    def _d(name):
        return name in vals
    try:
        r = eval(py, {"__builtins__": {}}, {"_v": _v, "_d": _d})  # noqa: S307 - expression built above from a closed grammar
    except Unknown:
        raise
    except Exception as e:
        raise Unknown(f"{s} ({e})")
    return bool(r.b() if isinstance(r, V) else r)


def holds(spec, vals):
    """spec = (base, task_whens); base is a when-list or ("ANY", [when-lists]) for a role
    reached by several paths: the requirement holds if ANY path's chain holds."""
    base, tw = spec
    chains = base[1] if isinstance(base, tuple) and base and base[0] == "ANY" else [base]
    seen_unknown = False
    for c in chains:
        h = chain_holds(list(c) + list(tw), vals)
        if h is True:
            return True
        if h is None:
            seen_unknown = True
    return None if seen_unknown else False


def chain_holds(whens, vals):
    """True / False / None (unknown) for an AND of when-clauses."""
    unknown = False
    for c in whens:
        try:
            if not evaluate(c, vals):
                return False
        except Unknown:
            unknown = True
    return None if unknown else True


# ------------------------------------------------------------------- main ---

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("site")
    ap.add_argument("--profile")
    ap.add_argument("--plan-file")
    ap.add_argument("--no-secrets", action="store_true")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--ansible-dir", help="an ops/ansible tree to scan instead of this repository's (fixtures, tests)")
    # The phases this scan speaks for. The standup gates twice: once before
    # phase 2 for everything up to the manual step, and once before phase 12
    # for what follows it. A secret the run itself issues in phase 11 (the
    # NetBird service token, which only phase 12 reads) is not missing when
    # phase 2 is about to run -- it does not exist yet, and cannot. Scanning
    # all fourteen phases up front refused every first run of a new goatnet.
    ap.add_argument("--from", dest="from_phase", type=int, default=0,
                    help="scan only phases numbered this or higher (default: all)")
    ap.add_argument("--to", dest="to_phase", type=int, default=0,
                    help="scan only phases numbered this or lower (default: all)")
    a = ap.parse_args()
    global ANS
    if a.ansible_dir:
        ANS = Path(a.ansible_dir).resolve()

    default_group = a.site.replace("-", "_") + "_hosts"
    phases = read_plan(a.site, a.profile, a.plan_file)
    if not phases:
        print(f"standup-required-inputs: no plan for {a.site}", file=sys.stderr)
        return 2

    # An observability leg is named, not numbered, and runs beside the phases;
    # a range names numbered phases, so the legs are left to the full scan.
    if a.from_phase or a.to_phase:
        lo, hi = a.from_phase or 0, a.to_phase or 10**6
        phases = [ph for ph in phases
                  if str(ph["phase"]).isdigit() and lo <= int(ph["phase"]) <= hi]
        if not phases:
            print(f"standup-required-inputs {a.site}: no phase in {lo}..{hi}")
            return 0

    missing, unsupplied, conditional, checked = {}, {}, {}, 0
    leaves, produced = {}, {}
    later = None
    if not a.no_secrets:
        p = posture()
        try:
            store = p.store(str(REPO)) if p else ""
        except Exception:  # a posture that cannot answer promises no phase 3
            store = ""
        later = LaterSteps(phases, a.site, store)
    by_group, skipped = {}, []
    for ph in phases:
        if ph["kind"] == "MANUAL":
            continue
        for play, group, extra, limit in plays_in(ph["cmd"], default_group):
            if limit and limit != group and (ANS / "group_vars" / limit).is_dir():
                group = limit   # the limit names another group: that group's hosts and vars apply
            elif limit and not limit_matches(limit, group):
                skipped.append((ph["phase"], play, limit, group))
                continue
            by_group.setdefault(group, []).append((ph, play, extra))

    for group, items in by_group.items():
        g = Graph()
        extra_all = {}
        for _ph, play, extra in items:
            g.add_play_file(play)
            extra_all.update(extra)
        site = group_vars(group)
        site.update(inventory_vars(group))
        site.setdefault("site_id", a.site)
        site.update(extra_all)
        defaults = role_defaults(g)
        play_vars = {}
        for _rel, play, _w in g.plays:
            if isinstance(play.get("vars"), dict):
                play_vars.update(play["vars"])
        known = dict(defaults)
        known.update(play_vars)
        known.update(site)
        where_phase = {play: ph for ph, play, _e in items}
        for var, need, where, whens in requirements(g):
            if var in MAGIC or var.startswith("ansible_") or var in g.set_at_runtime or var in g.params:
                continue
            checked += 1
            h = holds(whens, known)
            if h is False:
                continue
            present = var in site or var in play_vars or (var in defaults and not empty(defaults[var]))
            val = site.get(var, play_vars.get(var, defaults.get(var)))
            ok = present and not (need == "nonempty" and empty(val))
            key = (group, var)
            if not ok:
                bucket = conditional if h is None else missing
                bucket.setdefault(key, set()).add(where)
                continue
            m = ENV_LOOKUP.search(str(val)) if isinstance(val, str) else None
            if m and not a.no_secrets and not env_supplied(m.group(1)):
                unsupplied.setdefault((group, var, m.group(1)), set()).add(where)
        _ = where_phase
        if not a.no_secrets:
            gl = _goat_leaf()
            for leaf, by in leaves_needed(g, known, str(site.get("site_id", a.site))):
                checked += 1
                found = gl.goat_leaf(leaf)
                if not Path(found).is_file():
                    leaves.setdefault((group, leaf, found, gl.goat_leaf_mint(leaf)), set()).add(by)
        # Every env-bound secret the reached plays and roles template, not only the asserted
        # ones — the same derivation deploy-secrets-needed.py makes for one component's deploy.
        if not a.no_secrets:
            words = set()
            for rel, _p, _w in g.plays:
                words |= set(WORD.findall((ANS / rel).read_text(errors="replace")))
            for name in g.roles:
                for f in (ANS / "roles" / name).rglob("*"):
                    if f.is_file():
                        try:
                            words |= set(WORD.findall(f.read_text(errors="replace")))
                        except OSError:
                            pass   # an unreadable file templates nothing this scan can see
            for var, val in site.items():
                if var in words and isinstance(val, str):
                    m = ENV_LOOKUP.search(val)
                    if m and not env_supplied(m.group(1)) and not any(k[1] == var for k in unsupplied):
                        if "default(" in val and "true)" not in val.replace(" ", ""):
                            continue   # lookup(...) | default('x'): a declared fallback, not a need
                        if re.search(r"""\)\s+or\s+['"][^'"]+['"]""", val):
                            continue   # lookup(...) or 'x' (group_vars/all.yml's ghcr_org, source_repo): the same declared fallback
                        unsupplied.setdefault((group, var, m.group(1)), set()).add("templated by a reached play or role")
            # Role defaults bind secrets too (goat-get's release token reads
            # GOAT_CLI_RELEASE_TOKEN, else GH_TOKEN, else GHCR_READ_TOKEN): the same
            # derivation the deploy drivers make, one per reached role.
            dsn = deploy_secrets_needed()
            if dsn:
                svars = {k: str(v) for k, v in site.items() if isinstance(v, (str, int))}
                pwords = set()
                for rel, _p, _w in g.plays:
                    pwords |= set(WORD.findall((ANS / rel).read_text(errors="replace")))
                overridden = set(site) | set(play_vars)
                for var, alts in dsn.role_default_bindings(str(REPO), sorted(g.roles), overridden, svars, pwords).items():
                    if not any(env_supplied(e) for e in alts):
                        unsupplied.setdefault((group, var, "|".join(alts)), set()).add("a reached role's defaults")

            # What this same run produces is not a gap, unless an earlier step reads it.
            for key in [k for k in unsupplied if k[0] == group]:
                _g, var, envs = key
                step = next((later.secrets[e] for e in envs.split("|") if e in later.secrets), None)
                if step is None:
                    continue
                first = first_read(items, later, lambda _g2, w, v=var: v in w)
                if first is not None and first < step[0]:
                    unsupplied[key].add(f"read before phase {step[1]['phase']}, which produces it")
                    continue
                produced[(group, "secret", envs, step[1]["phase"], step[1]["label"])] = True
                del unsupplied[key]
            if later.mint is not None:
                ours = minted_leaves(str(site.get("site_id", a.site)), later.mint[1]["cmd"], known)
                for key in [k for k in leaves if k[0] == group and k[1] in ours]:
                    _g, leaf, _found, _cmd = key
                    first = first_read(items, later, lambda g2, _w, lf=leaf: lf in [n for n, _b in leaves_needed(g2, known, str(site.get("site_id", a.site)))])
                    if first is not None and first < later.mint[0]:
                        leaves[key].add(f"read before phase {later.mint[1]['phase']}, which mints it")
                        continue
                    produced[(group, "leaf", leaf, later.mint[1]["phase"], later.mint[1]["label"])] = True
                    del leaves[key]

    for (g, var) in list(conditional):
        if (g, var) in missing:
            missing[(g, var)] |= conditional.pop((g, var))

    missing_rows = [{"group": g, "var": v, "required_by": sorted(w)} for (g, v), w in sorted(missing.items())]
    # Names only: the variable a site sets and the environment name it reads. No value is read here.
    unsupplied_rows = [{"group": g, "var": v, "env": e, "required_by": sorted(w)} for (g, v, e), w in sorted(unsupplied.items())]
    conditional_rows = [{"group": g, "var": v, "required_by": sorted(w)} for (g, v), w in sorted(conditional.items())]
    skipped_rows = [{"phase": p, "play": pl, "limit": lim, "group": gr} for p, pl, lim, gr in skipped]
    # Paths only: where the leaf belongs and the command that mints it. No key is read.
    leaf_rows = [{"group": g, "leaf": lf, "looked": at, "mint": cmd, "required_by": sorted(w)}
                 for (g, lf, at, cmd), w in sorted(leaves.items())]
    # Names only, as above: what a later step of this run produces, and which step.
    produced_rows = [{"group": g, "kind": k, "name": n, "phase": ph, "step": lb}
                     for g, k, n, ph, lb in sorted(produced, key=lambda r: (int(r[3]) if r[3].isdigit() else 99, r[1], r[2]))]
    if a.json:
        print(json.dumps({"site": a.site, "checked": checked, "missing": missing_rows, "secrets": unsupplied_rows,
                          "conditional": conditional_rows, "skipped": skipped_rows, "leaves": leaf_rows,
                          "produced": produced_rows}, indent=2))
    else:
        for r in missing_rows:
            print(f"FAIL  input  {r['group']}: {r['var']} is required and not set — required by {', '.join(r['required_by'][:3])}")
        for r in unsupplied_rows:
            print(f"FAIL  secret {r['group']}: {r['var']} reads env {r['env']}, which the secrets posture does not supply — required by {', '.join(r['required_by'][:2])}")
        for r in leaf_rows:
            print(f"FAIL  leaf   {r['group']}: GB-071 {r['leaf']} is not at {r['looked']} — mint it: {r['mint']} (required by {', '.join(r['required_by'])})")
        for r in conditional_rows:
            print(f"WARN  maybe  {r['group']}: {r['var']} is required under a condition this scan cannot evaluate — {', '.join(r['required_by'][:2])}")
        for r in skipped_rows:
            print(f"note  skip   phase {r['phase']} {r['play']}: --limit {r['limit']} selects no host of {r['group']}, so the play runs on nothing")
        for r in produced_rows:
            print(f"later {r['kind']:<6} {r['group']}: {r['name']} is produced by phase {r['phase']} of this run ({r['step']})")
        print(f"standup-required-inputs {a.site}: {checked} requirements checked across {len(by_group)} group(s); "
              f"{len(missing_rows)} input(s), {len(unsupplied_rows)} secret(s) and {len(leaf_rows)} leaf file(s) missing, "
              f"{len(produced_rows)} produced by a later phase of this run, {len(conditional_rows)} conditional")
        if missing_rows:
            print("fix: set each input above in ops/ansible/group_vars/<group>/ (or the inventory) — every gap is listed, so one pass fixes them all")
        if unsupplied_rows:
            p = posture()
            try:
                hint = p.hint(sorted({r["env"] for r in unsupplied_rows}), str(REPO)) if p else []
            except Exception:  # a posture that cannot answer falls back to the generic fix line below
                hint = []
            print("fix: " + (" ".join(hint) if hint else "supply each secret above under your secrets posture (ops/scripts/with-secrets <command>)"))
    return 1 if (missing_rows or unsupplied_rows or leaf_rows) else 0


if __name__ == "__main__":
    sys.exit(main())
