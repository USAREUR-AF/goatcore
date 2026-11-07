# SPDX-License-Identifier: Apache-2.0
#
# resolve.py — the ONE resolution of intent out of the committed tree.
#
# Two precedences live here and nowhere else:
#
#   pins  — member deviation -> goatnet deviation -> fleet value
#           (ops/pins/fleet-pins.yaml, merged with the lite sidecar
#           ops/pins/lite-pins.yaml when it exists — see load_tables)
#   vars  — member group_vars -> group_vars/all.yml -> role default
#           (ops/ansible/group_vars/, ops/ansible/roles/*/defaults/main.yml)
#
# Both were reachable only by importing a hyphenated script through
# importlib.util.spec_from_file_location before CF-2. That import was the tell:
# this was always library code wearing a script's clothes, and the second
# consumer is where the copy gets made. Canon:
# docs/design/consistency-foundation.md §4.1 (ADR 1110 D2).
#
# NOTHING HERE GRADES. Resolution says what a value is and how confidently it
# was read; grading.py decides what that means. Keeping them apart is what lets
# a projection choose what it can see without letting it choose what a
# difference means.

import collections
import glob
import os
import re

import yaml

from .paths import ALLOWLIST, CAPS_FILE, GROUP_VARS, ROLES, TABLE, rel

# Keys the pin table governs: image pins and release tags. Deliberately NOT
# `*_version` — a version string (netbird_version: 0.75.1) is not a deploy pin
# and has its own consumers.
PIN_KEY_RE = re.compile(r"(_image|_release_tag)$")
DIGEST_RE = re.compile(r"@sha256:[0-9a-f]{64}$")

GENERATED_BASENAME = "pins.yml"

# ADR 1060's artifact-type dispatch. It travels in IntentV1 so a consumer can
# emit the right undo verb: an `image` rolls back, `data` and `cert` refuse and
# point at restore / rotate instead.
ARTIFACT_TYPES = ("image", "config", "acl", "data", "cert", "forward-only")
DEFAULT_ARTIFACT_TYPE = "image"

TRUTHY = {"true", "yes", "on"}
FALSY = {"false", "no", "off"}


class SchemaError(Exception):
    """An authoring surface is not usable as one. Raised rather than exited:
    a library that calls sys.exit cannot be tested, and the corpus is the point.
    The CLI entry points turn this back into `ERROR: …` + exit 2."""


def die(msg):
    raise SchemaError(msg)


# ---------------------------------------------------------------------------
# the pin table
# ---------------------------------------------------------------------------

LITE_BASENAME = "lite-pins.yaml"

# The merged pin table: the fleet table plus the lite sidecar. `lite_goatnets`
# is the set of goatnet names the sidecar declared — the intent builder needs
# it to know which `sources` a goatnet's generation was rendered from.
PinTable = collections.namedtuple("PinTable", "goatnets components lite_goatnets table lite_table")


def lite_table_for(path):
    """The sidecar beside a fleet table: ops/pins/lite-pins.yaml beside
    ops/pins/fleet-pins.yaml, and the same beside a scratch copy — so a test or
    a parity run on a copied tree reads ITS copy, never the checkout's."""
    return os.path.join(os.path.dirname(os.path.abspath(path)), LITE_BASENAME)


def load_lite(path, goatnets, components):
    """Read the lite sidecar and merge it into the fleet table IN PLACE.

    A lite goatnet (ADR 1213) is declared here and never in fleet-pins.yaml,
    so rendering one changes no byte of the fleet table and no committed
    intent's `sources` hash. The sidecar carries two things only: `goatnets:`
    rows (the fleet shape) and, under `components:`, `deviations:` blocks for
    those goatnets or their members. Component DEFINITIONS (`var:`, `fleet:`,
    the join keys) live in fleet-pins.yaml, and a sidecar that tries to carry
    one is refused. Merge rules: goatnets union, a name in both is an error;
    deviations union per component, a scope in both is an error.

    Returns the set of goatnet names the sidecar declared. A missing sidecar
    is an empty one."""
    if not os.path.exists(path):
        return set()
    with open(path) as f:
        data = yaml.safe_load(f) or {}
    where = rel(path)
    if not isinstance(data, dict):
        die("%s: must be a mapping with `goatnets:` and `components:`" % where)
    extra = sorted(set(data) - {"goatnets", "components"})
    if extra:
        die("%s: unknown top-level key(s) %s — the sidecar carries `goatnets:` and "
            "`components:` (deviations only)" % (where, ", ".join(extra)))
    lite_goatnets = data.get("goatnets") or {}
    lite_components = data.get("components") or {}
    if not isinstance(lite_goatnets, dict):
        die("%s: `goatnets:` must be a mapping" % where)
    if not isinstance(lite_components, dict):
        die("%s: `components:` must be a mapping" % where)

    for gn, spec in lite_goatnets.items():
        if gn in goatnets:
            die("%s: goatnet '%s' is also declared in %s — a goatnet lives in one "
                "table" % (where, gn, rel(TABLE)))
        goatnets[gn] = spec
    lite_scopes = set(lite_goatnets)
    for spec in lite_goatnets.values():
        lite_scopes.update((spec or {}).get("members") or [])

    for cname, spec in lite_components.items():
        spec = spec or {}
        if cname not in components:
            die("%s: component '%s' is not declared in %s — component definitions "
                "live in fleet-pins.yaml; the sidecar carries deviations only"
                % (where, cname, rel(TABLE)))
        other = sorted(set(spec) - {"deviations"})
        if other:
            die("%s: component '%s' carries %s — component definitions live in "
                "fleet-pins.yaml; the sidecar carries `deviations:` only"
                % (where, cname, ", ".join("`%s:`" % k for k in other)))
        devs = spec.get("deviations") or {}
        if not isinstance(devs, dict):
            die("%s: component '%s': `deviations:` must be a mapping" % (where, cname))
        target = components[cname]
        if target.get("deviations") is None:
            target["deviations"] = {}
        for scope, dev in devs.items():
            if scope not in lite_scopes:
                die("%s: component '%s': deviation scope '%s' is neither a goatnet "
                    "declared in this sidecar nor one of its members"
                    % (where, cname, scope))
            if scope in target["deviations"]:
                die("%s: component '%s': deviation '%s' is also declared in %s"
                    % (where, cname, scope, rel(TABLE)))
            target["deviations"][scope] = dev
    return set(lite_goatnets)


def load_tables(path=None, lite_path=None):
    """The merged pin table as a PinTable. `path` defaults to the committed
    fleet table; the sidecar defaults to lite-pins.yaml BESIDE whichever fleet
    table is read."""
    path = path or TABLE
    lite_path = lite_path or lite_table_for(path)
    with open(path) as f:
        data = yaml.safe_load(f) or {}
    goatnets = data.get("goatnets") or {}
    components = data.get("components") or {}
    if not isinstance(goatnets, dict):
        die("fleet-pins.yaml: `goatnets:` must be a mapping")
    if not isinstance(components, dict) or not components:
        die("fleet-pins.yaml: `components:` must be a non-empty mapping")
    lite = load_lite(lite_path, goatnets, components)
    # A checkout whose only goatnet is a goatlite (an open-source export) declares
    # none in fleet-pins.yaml and one in the sidecar; the merged table must not be empty.
    if not goatnets:
        die("fleet-pins.yaml + lite-pins.yaml: no goatnet is declared in either table")
    return PinTable(goatnets, components, frozenset(lite), path, lite_path)


def load_table(path=None):
    """(goatnets, components) of the merged table — the two-tuple every
    pre-sidecar caller unpacks."""
    t = load_tables(path)
    return t.goatnets, t.components


def validate(goatnets, components):
    """Schema + policy validation. Returns member -> goatnet."""
    member_of = {}
    for gn, spec in goatnets.items():
        members = (spec or {}).get("members") or []
        if not members:
            die("goatnet '%s' declares no members" % gn)
        for m in members:
            if m in member_of:
                die("member '%s' is claimed by both '%s' and '%s'" % (m, member_of[m], gn))
            if not os.path.isdir(os.path.join(GROUP_VARS, m)):
                die("goatnet '%s' lists member '%s', but %s/%s/ does not exist"
                    % (gn, m, rel(GROUP_VARS), m))
            member_of[m] = gn

    for name, gn in member_of.items():
        if name in goatnets:
            die("name '%s' is both a goatnet and a member host-group; deviation "
                "keys would be ambiguous" % name)

    seen_vars = {}
    for cname, spec in components.items():
        spec = spec or {}
        var = spec.get("var")
        if not var:
            die("component '%s' has no `var:`" % cname)
        if var in seen_vars:
            die("var '%s' is declared by both component '%s' and '%s'"
                % (var, seen_vars[var], cname))
        seen_vars[var] = cname
        if "fleet" not in spec:
            die("component '%s' has no `fleet:` (use `fleet: null` when there is "
                "no value of record yet)" % cname)

        fleet = spec["fleet"]
        if fleet is not None:
            check_ref(cname, "fleet", fleet, allow_mutable=False)

        devs = spec.get("deviations") or {}
        if not isinstance(devs, dict):
            die("component '%s': `deviations:` must be a mapping" % cname)
        for scope, dev in devs.items():
            dev = dev or {}
            if scope not in goatnets and scope not in member_of:
                die("component '%s': deviation scope '%s' is neither a listed "
                    "goatnet nor a member host-group" % (cname, scope))
            if not str(dev.get("reason") or "").strip():
                die("component '%s': deviation '%s' has no `reason:` — a "
                    "deviation without a reason is a schema error "
                    "(design doc §2 principle 2)" % (cname, scope))
            if dev.get("absent"):
                if "value" in dev:
                    die("component '%s': deviation '%s' sets both `absent:` and "
                        "`value:`" % (cname, scope))
                continue
            if "value" not in dev:
                die("component '%s': deviation '%s' has neither `value:` nor "
                    "`absent: true`" % (cname, scope))
            # A mutable tag is legal HERE and nowhere else: the reason says why.
            check_ref(cname, "deviation '%s'" % scope, dev["value"], allow_mutable=True)

        # CF-4: the declared render watch-set. A list of on-host paths (which
        # may embed `{{ vars }}` — carried literal, resolved by ansible at
        # record time), and it needs a ledger home: the render manifest lives
        # under /var/lib/goat/<ledger_component>/, so a component declared
        # ledger-uncovered has nowhere to record what it rendered.
        watch = spec.get("render_watch")
        if watch is not None:
            if not isinstance(watch, list) or not watch:
                die("component '%s': `render_watch:` must be a non-empty list of "
                    "on-host file paths" % cname)
            for p in watch:
                if not isinstance(p, str) or not p.strip():
                    die("component '%s': `render_watch:` entries must be non-empty "
                        "path strings" % cname)
            if len(set(watch)) != len(watch):
                die("component '%s': `render_watch:` lists a path twice" % cname)
            if spec.get("ledger_component", cname) is None:
                die("component '%s' declares `render_watch:` but `ledger_component: "
                    "null` — the render manifest lives under /var/lib/goat/"
                    "<ledger_component>/, so a component with no ledger home has "
                    "nowhere to record its renders. Give it a deploying role "
                    "(ADR 1060 coverage) first." % cname)

        atype = spec.get("artifact_type") or DEFAULT_ARTIFACT_TYPE
        if atype not in ARTIFACT_TYPES:
            die("component '%s': artifact_type '%s' is not one of %s (ADR 1060's "
                "dispatch set — the artifact type decides whether undo means "
                "rollback, restore, rotate or refuse)"
                % (cname, atype, ", ".join(ARTIFACT_TYPES)))

        # `fleet: null` is only resolvable if every listed goatnet deviates.
        if fleet is None:
            for gn, spec_gn in goatnets.items():
                members = (spec_gn or {}).get("members") or []
                for m in members:
                    if m not in devs and gn not in devs:
                        die("component '%s' has `fleet: null` but '%s' (goatnet "
                            "'%s') carries no deviation — the pin is unresolvable"
                            % (cname, m, gn))
    return member_of


def check_ref(cname, where, ref, allow_mutable):
    if not isinstance(ref, str) or not ref.strip():
        die("component '%s' %s: image ref must be a non-empty string" % (cname, where))
    if DIGEST_RE.search(ref):
        return
    if not allow_mutable:
        die("component '%s' %s: '%s' is a mutable/bare tag. Pins must carry a "
            "multi-arch INDEX digest (@sha256:…, F-245). A mutable tag is legal "
            "only inside a deviation stanza whose reason says why."
            % (cname, where, ref))


def resolve(spec, member, goatnet):
    """member deviation → goatnet deviation → fleet. Returns (value, deviation|None)."""
    devs = spec.get("deviations") or {}
    for scope in (member, goatnet):
        if scope in devs:
            dev = devs[scope] or {}
            if dev.get("absent"):
                return None, dev
            return dev["value"], dev
    return spec.get("fleet"), None


def deviation_scope(spec, member, goatnet):
    """Which scope key a member's deviation was written under, or None."""
    devs = spec.get("deviations") or {}
    for scope in (member, goatnet):
        if scope in devs:
            return scope
    return None


def governed_vars(components):
    return {spec["var"] for spec in components.values() if spec and spec.get("var")}


def artifact_type(spec):
    return (spec or {}).get("artifact_type") or DEFAULT_ARTIFACT_TYPE


# ---------------------------------------------------------------------------
# the group_vars tree
# ---------------------------------------------------------------------------

class Resolution(object):
    """One key resolved for one member, with its resolution STATE attached.

    `state` is `resolved` / `unset` / `unreadable`, and only `resolved` may ever
    contribute to a `match`. The two non-resolved states are kept APART on
    purpose: `unset` means there was nothing to read, `unreadable` means it was
    read and could not be evaluated. Collapsing them would let a missing key
    hide behind a templating excuse."""

    __slots__ = ("display", "key", "state", "source", "detail")

    def __init__(self, display, key, state, source=None, detail=None):
        self.display = display
        self.key = key
        self.state = state
        self.source = source
        self.detail = detail


def read_yaml(path):
    with open(path) as f:
        data = yaml.safe_load(f) or {}
    return data if isinstance(data, dict) else {}


def scope_vars(directory):
    """key -> (value, source path). Ansible loads a group_vars dir alphabetically
    and the LATER file wins on a key conflict; this mirrors that."""
    out = {}
    for path in sorted(glob.glob(os.path.join(directory, "*.yml"))):
        for k, v in read_yaml(path).items():
            out[k] = (v, rel(path))
    return out


def role_defaults():
    """key -> list of (role, value), for the last-resort resolution layer."""
    out = {}
    for path in sorted(glob.glob(os.path.join(ROLES, "*", "defaults", "main.yml"))):
        role = os.path.basename(os.path.dirname(os.path.dirname(path)))
        for k, v in read_yaml(path).items():
            out.setdefault(k, []).append((role, v))
    return out


def all_scope_vars():
    """group_vars/all.yml as a scope map, labelled by its repo-relative path."""
    all_path = os.path.join(GROUP_VARS, "all.yml")
    if not os.path.exists(all_path):
        return {}
    return {k: (v, rel(all_path)) for k, v in read_yaml(all_path).items()}


def member_scopes(members):
    return {m: scope_vars(os.path.join(GROUP_VARS, m)) for m in members}


def resolve_var_full(var, member_scope, all_scope, defaults):
    """Ansible-ish precedence: member group_vars -> group_vars/all -> role default."""
    for scope, label in ((member_scope, None), (all_scope, "group_vars/all.yml")):
        if var in scope:
            value, source = scope[var]
            return render_value(value, label or source)

    hits = defaults.get(var) or []
    if len(hits) == 1:
        role, value = hits[0]
        return render_value(value, "role default: %s" % role)
    if len(hits) > 1:
        roles = ", ".join(sorted(r for r, _ in hits))
        return Resolution("unreadable (defined as a default by %d roles: %s — no single "
                          "default to attribute)" % (len(hits), roles), None, "unreadable",
                          detail="defined as a default by %d roles (%s) — no single default "
                                 "to attribute" % (len(hits), roles))
    return Resolution("unset (no group_vars entry, no role default)", None, "unset",
                      detail="no group_vars entry, no role default")


def resolve_var(var, member_scope, all_scope, defaults):
    """The (display, key, state) triple the markdown renderer consumes."""
    r = resolve_var_full(var, member_scope, all_scope, defaults)
    return r.display, r.key, r.state


def render_value(value, source):
    if isinstance(value, bool):
        return Resolution("%s (%s)" % (str(value).lower(), source), str(value).lower(),
                          "resolved", source=source)
    if isinstance(value, str):
        # A templated or env-sourced value is read-but-not-evaluable, and its
        # VALUE never travels: `lookup('env', …)` is how a secret would reach
        # the intent artifact, and that artifact is committed and served.
        if "{{" in value or "lookup(" in value:
            return Resolution("unreadable (templated; %s)" % source, None, "unreadable",
                              source=source, detail="templated")
        low = value.strip().lower()
        if low in TRUTHY or low in FALSY:
            norm = "true" if low in TRUTHY else "false"
            return Resolution("%s (%s, as the string %r)" % (norm, source, value), norm,
                              "resolved", source=source)
        return Resolution("%s (%s)" % (value, source), value, "resolved", source=source)
    if value is None:
        return Resolution("null (%s)" % source, "null", "resolved", source=source)
    return Resolution("%s (%s)" % (value, source), str(value), "resolved", source=source)


# ---------------------------------------------------------------------------
# the curated capability half, and the ungoverned tail
# ---------------------------------------------------------------------------

def load_caps(path=None):
    path = path or CAPS_FILE
    with open(path) as f:
        data = yaml.safe_load(f) or {}
    for section in ("capabilities", "node_shape", "retired"):
        rows = data.get(section) or []
        if not isinstance(rows, list):
            die("%s: `%s:` must be a list" % (rel(path), section))
        for row in rows:
            if not (row or {}).get("var"):
                die("%s: a `%s:` row has no `var:`" % (rel(path), section))
            if not str((row or {}).get("why") or "").strip():
                die("%s: `%s` row '%s' has no `why:` — a tracked row must say "
                    "why it earns one" % (rel(path), section, row.get("var")))
    devs = data.get("deviations") or {}
    if not isinstance(devs, dict):
        die("%s: `deviations:` must be a mapping" % rel(path))
    for var, scopes in devs.items():
        for scope, dev in (scopes or {}).items():
            if not str((dev or {}).get("reason") or "").strip():
                die("%s: deviation %s/%s has no `reason:` — a deviation without "
                    "a reason is a schema error (design doc §2 principle 2)"
                    % (rel(path), var, scope))
    return data


def load_allowlist(path=None):
    """The shrink-only unmigrated-pin tail. Returns (rows, count)."""
    path = path or ALLOWLIST
    if not os.path.exists(path):
        return set(), 0
    rows = set()
    with open(path) as f:
        for raw in f:
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            if ":" not in line:
                die("%s: malformed row '%s' (want <member_group>:<var>)" % (rel(path), line))
            rows.add(line)
    return rows, len(rows)


def load_baseline(path=None):
    """The shrink-only tail's declared ceiling: the `# baseline: <n>` line in
    the allowlist file itself, so the number and the rows it bounds share one
    home (read by generate-pins.py and `goatbuilder pins` alike). None when the
    file declares none."""
    path = path or ALLOWLIST
    if not os.path.exists(path):
        return None
    with open(path) as f:
        for raw in f:
            tail = raw.strip()
            if tail.startswith("# baseline:"):
                word = tail[len("# baseline:"):].split()
                if word and word[0].isdigit():
                    return int(word[0])
    return None


def scan_outside_pins(member_of):
    """Governed-shaped pin keys defined outside pins.yml in a listed member."""
    found = []
    for member in sorted(member_of):
        d = os.path.join(GROUP_VARS, member)
        for path in sorted(glob.glob(os.path.join(d, "*.yml"))):
            if os.path.basename(path) == GENERATED_BASENAME:
                continue
            data = read_yaml(path)
            for k in data:
                if PIN_KEY_RE.search(k):
                    found.append((member, k, rel(path)))
    return found
