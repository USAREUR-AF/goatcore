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
ARTIFACT_TYPES = ("image", "config", "acl", "data", "cert", "forward-only", "release")
# The architectures a `release` component names a file for, and the keys of
# its `release:` declaration (gate J23).
RELEASE_ARCHES = ("amd64", "arm64")
RELEASE_KEYS = ("bundle", "checksums", "files", "play", "signer")
# The signature formats of `release.bundle`; the first is the default.
RELEASE_BUNDLES = ("cosign", "sigstore", "attestation")
DEFAULT_ARTIFACT_TYPE = "image"

# An opt-in component (`opt_in: true`) runs only on a goatnet that names it in its
# own row's `opt_in:` list. Every other goatnet does not resolve, deploy or verify
# it, and leaves it out of its intent, so a new opt-in row changes no existing
# goatnet's resolved components (only `sources`, the table's own hash, moves).
# load_tables records which goatnets opted in under OPTED_KEY, which no table may
# write itself.
OPTED_KEY = "_opted_in"


def apply_opt_in(goatnets, components):
    """Record on each opt-in component the goatnets whose row names it, and refuse
    an `opt_in:` list that names anything but an opt-in component. Idempotent:
    validate() calls it again on whatever table it is handed."""
    for gn, gspec in goatnets.items():
        listed = (gspec or {}).get("opt_in")
        if listed is None:
            continue
        if not isinstance(listed, list) or not all(isinstance(c, str) for c in listed):
            die("goatnet '%s': `opt_in:` must be a list of component names" % gn)
        for cname in listed:
            spec = components.get(cname)
            if not isinstance(spec, dict):
                die("goatnet '%s': `opt_in:` names '%s', which is not a component" % (gn, cname))
            if spec.get("opt_in") is not True:
                die("goatnet '%s': `opt_in:` names '%s', which is not an opt-in component "
                    "(`opt_in: true`)" % (gn, cname))
    for cname, spec in components.items():
        if isinstance(spec, dict) and spec.get("opt_in") is True:
            spec[OPTED_KEY] = sorted(gn for gn, g in goatnets.items()
                                     if cname in ((g or {}).get("opt_in") or []))


def opted_out(spec, goatnet):
    """True when `spec` is an opt-in component that `goatnet` did not opt in to."""
    spec = spec or {}
    return spec.get("opt_in") is True and goatnet not in (spec.get(OPTED_KEY) or ())

# Placement (goatnet-deployment-doctrine.md P13). A component is single-homed
# only while it is an admin write class ADR 1165 has not yet lifted, and it says
# so by declaring `single_homed_role:`. A deviation reason that claims
# single-homing for a component that declares none is the wrong reason for an
# absence; a component declared `read_only: true` is never single-homed.
SINGLE_HOMING_CLAIM_RE = re.compile(r"single[- ]?homed|\bD3\b", re.IGNORECASE)


VAR_NAME_RE = re.compile(r"^[a-z_][a-z0-9_]*$")


def check_one_host(cname, spec):
    """`one_host_var:` names the group variable that names the one host of each
    member group the component runs on (goatbuilder push::placement)."""
    if "one_host_var" not in spec:
        return
    var = spec["one_host_var"]
    if not isinstance(var, str) or not VAR_NAME_RE.match(var):
        die("component '%s': `one_host_var:` is the name of a group variable that names "
            "one host, such as example_host" % cname)
    if spec.get("single_homed_role"):
        die("component '%s' declares both `one_host_var:` and `single_homed_role:`; "
            "a component runs on one host of each group by one of them" % cname)


def check_placement(components):
    """The two P13 placement rules, raised as SchemaError."""
    for cname, spec in components.items():
        spec = spec or {}
        role = spec.get("single_homed_role")
        read_only = spec.get("read_only")
        if read_only is not None and not isinstance(read_only, bool):
            die("component '%s': `read_only:` must be true or false" % cname)
        if role is not None and (not isinstance(role, str) or not role.strip()):
            die("component '%s': `single_homed_role:` must be a router_role "
                "letter such as \"A\"" % cname)
        if read_only and role:
            die("component '%s' declares both `read_only: true` and "
                "`single_homed_role: %s`. A read-only surface runs at every site "
                "of its goatnet and is never single-homed "
                "(goatnet-deployment-doctrine.md P13)." % (cname, role))
        if role:
            continue
        for scope, dev in (spec.get("deviations") or {}).items():
            reason = " ".join(str((dev or {}).get("reason") or "").split())
            hit = SINGLE_HOMING_CLAIM_RE.search(reason)
            if hit:
                die("component '%s': deviation '%s' gives single-homing as its "
                    "reason (\"%s\"), but the component declares no "
                    "`single_homed_role:`. Only an admin write class not yet "
                    "lifted by ADR 1165 is single-homed, and it declares the "
                    "role. Declare it, or state the real reason for the "
                    "deviation (goatnet-deployment-doctrine.md P13)."
                    % (cname, scope, hit.group(0)))


TRUTHY = {"true", "yes", "on"}
FALSY = {"false", "no", "off"}


class SchemaError(Exception):
    """An authoring surface is not usable as one. Raised rather than exited:
    a library that calls sys.exit cannot be tested, and the corpus is the point.
    The CLI entry points turn this back into `ERROR: …` + exit 2."""


# ADR 1256: a member deviation names the goatnet value it overrides. Without
# it, a member deviation written before a goatnet-level change (a hotfix
# deviation, a graduation, a fleet bump) keeps winning by precedence, and the
# member silently misses the change. `over:` is an optimistic lock written in
# the table: it must equal what the member's goatnet resolves the component to
# without the member deviation. A ref, or `absent` (the goatnet declares the
# component absent), or `none` (`fleet: null` and no goatnet deviation).
OVER_ABSENT = "absent"
OVER_NONE = "none"


def goatnet_resolution(spec, goatnet):
    """What `goatnet` resolves a component to with no member deviation, as the
    `over:` token: the goatnet deviation's value, `absent`, or the fleet value
    (`none` when fleet is null)."""
    dev = (spec.get("deviations") or {}).get(goatnet)
    if dev is not None:
        dev = dev or {}
        if dev.get("absent"):
            return OVER_ABSENT
        return str(dev.get("value"))
    fleet = spec.get("fleet")
    return OVER_NONE if fleet is None else str(fleet)


def over_line(token):
    """The line an operator pastes under the member deviation."""
    if token in (OVER_ABSENT, OVER_NONE):
        return "over: %s" % token
    return 'over: "%s"' % token


def check_member_over(goatnets, components, member_of):
    """GB-358: a member `value:` deviation must carry `over:`, and only it may.
    GB-357: `over:` must name what the member's goatnet resolves to now."""
    for cname, spec in components.items():
        spec = spec or {}
        for scope, dev in (spec.get("deviations") or {}).items():
            dev = dev or {}
            has_over = "over" in dev
            if scope in goatnets:
                if has_over:
                    die("GB-358 pins: component '%s': `over:` is on the goatnet deviation "
                        "'%s'. Only a member deviation names what it overrides; a goatnet "
                        "deviation overrides fleet. Delete the `over:` line."
                        % (cname, scope))
                continue
            if dev.get("absent"):
                if has_over:
                    die("GB-358 pins: component '%s': member deviation '%s' is `absent: "
                        "true` and also carries `over:`. A member that does not run the "
                        "component shadows nothing. Delete the `over:` line."
                        % (cname, scope))
                continue
            goatnet = member_of.get(scope)
            if goatnet is None:
                continue
            now = goatnet_resolution(spec, goatnet)
            value = dev.get("value")
            if not has_over:
                die("GB-358 pins: component '%s': member deviation '%s' pins %s but does "
                    "not name what it overrides. Goatnet '%s' resolves '%s' to %s without "
                    "it. To keep %s on %s deliberately, add under deviations.%s:  %s  "
                    "Or delete the deviation, and %s takes %s."
                    % (cname, scope, value, goatnet, cname, now, value, scope, scope,
                       over_line(now), scope, now))
            if str(dev.get("over")) != now:
                die("GB-357 pins: component '%s': member deviation '%s' was written over "
                    "%s, but goatnet '%s' now resolves '%s' to %s, so %s would silently "
                    "keep %s. Delete the deviation, and %s takes %s. Or, to keep %s on %s "
                    "deliberately, set under deviations.%s:  %s"
                    % (cname, scope, dev.get("over"), goatnet, cname, now, scope, value,
                       scope, now, value, scope, scope, over_line(now)))


def redundant_member_deviations(goatnets, components, member_of):
    """Member value deviations that pin what their goatnet already resolves
    to: a warning, not a refusal (it shadows nothing today, and becomes a
    shadow the moment the goatnet value moves)."""
    out = []
    for cname, spec in sorted(components.items()):
        spec = spec or {}
        for scope, dev in sorted((spec.get("deviations") or {}).items()):
            dev = dev or {}
            if scope in goatnets or dev.get("absent") or scope not in member_of:
                continue
            now = goatnet_resolution(spec, member_of[scope])
            if str(dev.get("value")) == now:
                out.append("WARN pins: component '%s': member deviation '%s' pins %s, "
                           "which goatnet '%s' already resolves to. It changes nothing "
                           "today and would hold %s back from the goatnet's next value. "
                           "Delete it." % (cname, scope, now, member_of[scope], scope))
    return out


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
    for cname, spec in components.items():
        if isinstance(spec, dict) and OPTED_KEY in spec:
            die("component '%s': `%s:` is not a key a table may set" % (cname, OPTED_KEY))
    lite = load_lite(lite_path, goatnets, components)
    apply_opt_in(goatnets, components)
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
    apply_opt_in(goatnets, components)
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
        check_one_host(cname, spec)
        if "opt_in" in spec and not isinstance(spec["opt_in"], bool):
            die("component '%s': `opt_in:` must be true or false" % cname)
        for scope, dev in devs.items():
            dev = dev or {}
            owner = scope if scope in goatnets else member_of.get(scope)
            if owner is not None and opted_out(spec, owner) and not dev.get("absent"):
                die("component '%s' is opt-in (`opt_in: true`), and '%s' sets a value for "
                    "goatnet '%s', which does not list it under `opt_in:`. Add it to that "
                    "goatnet's `opt_in:` list, or drop the deviation" % (cname, scope, owner))
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
        check_source(cname, spec)
        if atype == "release":
            check_release(cname, spec, fleet, devs)
        elif "release" in spec:
            die("component '%s' declares `release:` but its artifact_type is '%s'; "
                "only a release component names a release" % (cname, atype))

        # `fleet: null` is only resolvable if every listed goatnet deviates.
        if fleet is None:
            for gn, spec_gn in goatnets.items():
                if opted_out(spec, gn):
                    continue
                members = (spec_gn or {}).get("members") or []
                for m in members:
                    if m not in devs and gn not in devs:
                        die("component '%s' has `fleet: null` but '%s' (goatnet "
                            "'%s') carries no deviation — the pin is unresolvable"
                            % (cname, m, gn))
    check_placement(components)
    check_member_over(goatnets, components, member_of)
    return member_of


# A release component's signer: the workflow that signs its releases, as
# GitHub names it in a certificate's SAN, without the ref (the pin's tag).
RELEASE_SIGNER_RE = re.compile(
    r"^https://github\.com/[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9._-]+/\.github/workflows/[A-Za-z0-9._-]+\.ya?ml\Z")
RELEASE_REF_RE = re.compile(
    r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9._-]+@[A-Za-z0-9][A-Za-z0-9._+-]*(?:/[A-Za-z0-9][A-Za-z0-9._+-]*)*@sha256:[0-9a-f]{64}\Z")


RELEASE_FILE_NAME_RE = re.compile(r"[A-Za-z0-9._+-]+")


def _release_file_name(v):
    """One path segment of [A-Za-z0-9._+-], the names the release-file role
    accepts on the host, and never the name goatbuilder keeps a file's
    attestations under."""
    return isinstance(v, str) and v not in (".", "..") and not v.endswith(".attestations.json") and bool(RELEASE_FILE_NAME_RE.fullmatch(v))


def signer_repository(signer):
    """The <owner>/<repository> a release signer names."""
    if not isinstance(signer, str) or not signer.startswith("https://github.com/"):
        return None
    parts = signer[len("https://github.com/"):].split("/")
    if len(parts) < 2 or not parts[0] or not parts[1]:
        return None
    return "%s/%s" % (parts[0], parts[1])


SOURCE_KEYS = ("repository", "commit", "tag", "signer", "play")


def source_ref_problem(src):
    """Why a `source:` mapping's commit, tag and signer pin nothing, if they do
    not. Mirrors goatbuilder's source::ref_problem word for word."""
    def has(k):
        # A key with no value is no key, as a type file's loader reads it.
        return src.get(k) is not None

    def text(k):
        return src[k] if isinstance(src.get(k), str) else ""
    if has("commit") and has("tag"):
        return "`source:` is pinned by a full commit or by a signed tag, not both"
    if has("signer") and not has("tag"):
        return "`source.signer:` is the key that signs `source.tag:`, and there is no tag"
    if not has("tag"):
        commit = text("commit")
        if (not re.fullmatch(r"[0-9a-f]{40}", commit) or re.fullmatch(r"[0-9]+([eE][0-9]+)?", commit)
                or re.fullmatch(r"0b[01]+", commit)):
            return "`source.commit:` is a full commit of 40 hexadecimal characters; a branch or a tag can move"
        return None
    tag = text("tag")
    if (not re.fullmatch(r"[A-Za-z][A-Za-z0-9._/-]*", tag) or tag.lower() in ("y", "n", "yes", "no", "true", "false", "on", "off")
            or ".." in tag or "//" in tag or tag.endswith("/") or tag.endswith(".lock")):
        return "`source.tag:` is the name of a tag"
    if not has("signer"):
        return ("`source.tag:` is pinned only by its signer: name `source.signer:`, the SSH public key "
                "that signs it, or pin a full commit; a tag no one's key is held to can move")
    if not re.fullmatch(r"(ssh-ed25519|ecdsa-sha2-nistp256|ecdsa-sha2-nistp384|ecdsa-sha2-nistp521"
                        r"|sk-ssh-ed25519@openssh\.com|sk-ecdsa-sha2-nistp256@openssh\.com) [A-Za-z0-9+/]+={0,3}",
                        text("signer")):
        return "`source.signer:` is an SSH public key, its type and its base64 text, with no comment"
    return None


def check_source(cname, spec):
    """A component whose play is in another repository (gate J26). Mirrors
    goatbuilder's source::problem word for word."""
    if "source" not in spec:
        return
    src = spec["source"]
    if not isinstance(src, dict):
        die("component '%s': `source:` is a mapping (repository, commit or tag and signer, play)" % cname)
    unknown = sorted(k for k in src if k not in SOURCE_KEYS)
    if unknown:
        die("component '%s': `source:` has an unknown key '%s' (known: %s)" % (cname, unknown[0], ", ".join(SOURCE_KEYS)))
    repository, play = (src[k] if isinstance(src.get(k), str) else "" for k in ("repository", "play"))
    if (not re.fullmatch(r"(https://|ssh://|file:///|[A-Za-z0-9._-]+@[A-Za-z0-9.-]+:)[A-Za-z0-9._~/:@-]+", repository)
            or re.match(r"https://[^/]*@", repository) or re.match(r"ssh://[^/@]*:[^/@]*@", repository)):
        die("component '%s': `source.repository:` is a git URL (https, ssh or file) with no credential in it" % cname)
    why = source_ref_problem(src)
    if why:
        die("component '%s': %s" % (cname, why))
    if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_./-]*\.ya?ml", play) or any(x in ("", ".", "..") for x in play.split("/")):
        die("component '%s': `source.play:` is a play's path inside the repository" % cname)
    if spec.get("deploy_playbook") is not None:
        die("component '%s' declares both deploy_playbook and source; its play is in one place" % cname)


def check_release(cname, spec, fleet, devs):
    """A `release` component (gate J23): its `release:` declaration and every
    pin it carries, `<owner>/<repository>@<tag>@sha256:<digest>`, the digest
    being that of the release's checksums file. Mirrors goatbuilder's
    pins::check_release word for word."""
    rel = spec.get("release")
    if not isinstance(rel, dict):
        die("component '%s' is artifact_type 'release' but has no `release:` "
            "mapping (checksums, files, signer)" % cname)
    unknown = sorted(k for k in rel if k not in RELEASE_KEYS)
    if unknown:
        die("component '%s': `release:` has an unknown key '%s' (known: %s)"
            % (cname, unknown[0], ", ".join(RELEASE_KEYS)))
    if not _release_file_name(rel.get("checksums")):
        die("component '%s': `release.checksums:` must name the release's "
            "checksums file, one file name" % cname)
    if "bundle" in rel and rel["bundle"] not in RELEASE_BUNDLES:
        die("component '%s': `release.bundle:` must be one of %s (the signature "
            "beside the checksums file)" % (cname, ", ".join(RELEASE_BUNDLES)))
    if "play" in rel:
        play = rel["play"] if isinstance(rel["play"], str) else ""
        if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_./-]*\.ya?ml", play) or any(x in ("", ".", "..") for x in play.split("/")):
            die("component '%s': `release.play:` is a play's path inside the release's archive" % cname)
        if "deploy_playbook" in spec or "source" in spec:
            die("component '%s' declares `release.play` and deploy_playbook or source; its play is in one place" % cname)
    files = rel.get("files")
    if not isinstance(files, dict) or not files:
        die("component '%s': `release.files:` must map each architecture (%s) "
            "to its file name" % (cname, ", ".join(RELEASE_ARCHES)))
    for arch in sorted(files):
        if arch not in RELEASE_ARCHES:
            die("component '%s': `release.files:` names architecture '%s' "
                "(known: %s)" % (cname, arch, ", ".join(RELEASE_ARCHES)))
        if not _release_file_name(files[arch]):
            die("component '%s': `release.files.%s:` must be one file name"
                % (cname, arch))
    signer = rel.get("signer")
    if not isinstance(signer, str) or not RELEASE_SIGNER_RE.fullmatch(signer):
        die("component '%s': `release.signer:` is the workflow that signs the release, https://github.com/<owner>/<repository>/.github/workflows/<file>.yml, with no ref: the pin's tag is its ref" % cname)
    refs = []
    if fleet is not None:
        refs.append(("fleet", fleet))
    for scope, dev in devs.items():
        if isinstance(dev, dict) and "value" in dev:
            refs.append(("deviation '%s'" % scope, dev["value"]))
    for where, v in refs:
        if not isinstance(v, str) or not RELEASE_REF_RE.fullmatch(v):
            die("component '%s' %s: '%s' is not a release pin "
                "<owner>/<repository>@<tag>@sha256:<digest of its checksums file>"
                % (cname, where, v))
        repo = v.split("@", 1)[0]
        srepo = signer_repository(signer)
        if srepo is None or srepo.lower() != repo.lower():
            die("component '%s' %s: the pin's repository '%s' is not the repository "
                "of `release.signer:` ('%s'): a release pin's repository must be its "
                "signer's repository" % (cname, where, repo, srepo or ""))


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
    if opted_out(spec, goatnet):
        return None, None
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


def governed_vars_for(components, goatnet):
    """The vars the table governs for `goatnet`'s members: an opt-in component's
    var is governed only where the goatnet opted in. Elsewhere the component is
    not deployed, so a hand pin of its var is not a shadowed generated pin."""
    return {spec["var"] for spec in components.values()
            if spec and spec.get("var") and not opted_out(spec, goatnet)}


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

# Rows keyed by these fields; an extension row with the same key replaces the
# row in place, a new key is appended.
CAPS_ROW_KEYS = (("capabilities", "var"), ("node_shape", "var"),
                 ("retired", "var"), ("cross_goatnet", "capability"))


def caps_files(path=None):
    """The capabilities file and its extensions, in merge order.

    A deployment supplies its own goatnets' rows in <file>.d/*.yaml beside it
    (for ops/pins/parity-capabilities.yaml: ops/pins/parity-capabilities.d/):
    the same sections, merged in file-name order. A `capabilities`/`node_shape`/
    `retired` row with the `var` of an existing row (a `cross_goatnet` row with
    its `capability`) replaces it; others are appended. Under `deviations`, a var
    an extension declares carries the extension's scopes only: the public file's
    deviations describe its example goatnets, which a deployment does not run."""
    path = path or CAPS_FILE
    ext = path[:-len(".yaml")] + ".d" if path.endswith(".yaml") else path + ".d"
    return [path] + sorted(glob.glob(os.path.join(ext, "*.yaml")))


def _check_caps(path, data):
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


def load_caps(path=None):
    files = caps_files(path)
    data = None
    for f in files:
        with open(f) as fh:
            part = yaml.safe_load(fh) or {}
        _check_caps(f, part)
        if data is None:
            data = part
            continue
        for section, key in CAPS_ROW_KEYS:
            rows = part.get(section) or []
            if not rows:
                continue
            base = list(data.get(section) or [])
            at = {(r or {}).get(key): i for i, r in enumerate(base)}
            for row in rows:
                k = (row or {}).get(key)
                if k in at:
                    base[at[k]] = row
                else:
                    at[k] = len(base)
                    base.append(row)
            data[section] = base
        if part.get("deviations"):
            devs = dict(data.get("deviations") or {})
            devs.update(part["deviations"])
            data["deviations"] = devs
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
