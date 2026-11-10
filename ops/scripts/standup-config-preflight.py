#!/usr/bin/env python3
"""standup-config-preflight — assert a goatnet's group_vars are standup-ready
BEFORE any play runs against a host.

Each config defect in a standup otherwise surfaces one-per-run at play runtime,
as serial site-bootstrap attempts. Every check below is one such defect; the
number in brackets is its row in the standup friction list.
Run it after editing group_vars, before the first ansible-playbook:
table). Run it after editing group_vars, before the first ansible-playbook:

    ops/scripts/standup-config-preflight.py site_a_hosts [--obs site_a_obs_hosts]

Exit 0 = all checks pass. Non-zero = numbered FAIL lines, each naming the
group_vars key to fix and the failure it prevents. Offline — reads only
ops/ansible/group_vars/; contacts no host.
"""
import argparse
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("standup-config-preflight: needs PyYAML (pip3 install pyyaml)")

REPO = Path(__file__).resolve().parents[2]
GV = REPO / "ops" / "ansible" / "group_vars"

FAILS = []
WARNS = []


def fail(msg):
    FAILS.append(msg)


def warn(msg):
    WARNS.append(msg)


# F-393 — the reserved exclusions key that says "this group is a DIFFERENT CLASS
# from the reference", rather than listing the reference's files one by one.
#
# It can be written in .groupvars-parity.yaml with good reasoning and still
# exempt nothing, because both the
# enforcing path and --scaffold-plan matched exclusions by FILENAME: `if f in
# excl`. A key that is not a filename never matches one. Worse, the stale-
# exclusion hygiene loop then fired on it — "names a file no longer in
# reference" — telling the author to delete the decision.
#
# The exemption is deliberately NOT silent. A class that waives twenty files is
# a bigger claim than a per-file exclusion, so it prints what it waived; a
# reviewer sees the count and the reason rather than a clean run.
GROUP_CLASS_KEY = "__group_class__"


def split_exclusions(excl):
    """(per-file exclusions, class reason or None) from one group's excl map."""
    reason = excl.get(GROUP_CLASS_KEY)
    files = {k: v for k, v in excl.items() if k != GROUP_CLASS_KEY}
    return files, reason


def resolve_reference(spec, override=None):
    """(reference group, None) when usable, else (None, why and what to do).

    The reference is one of the operator's own goatnets: its group_vars file set
    is what a new site is checked and scaffolded against. A fresh checkout
    declares none (`reference: ''`), so "none declared" and "declared but not
    found" are told apart and each says how to proceed.
    """
    ref = override or spec.get("reference")
    if not ref:
        return None, (
            "no reference goatnet is declared: `reference:` in "
            "group_vars/.groupvars-parity.yaml is empty. Set it to the group_vars "
            "directory of a goatnet you already run (e.g. reference: <site>_hosts), "
            "or pass --scaffold-from <group> for one run. A goatlite site "
            "(preset: lite) needs neither: ops/scripts/goatlite-render.py writes "
            "its whole config surface.")
    if not (GV / ref).is_dir():
        return None, (
            f"reference '{ref}' has no directory under group_vars/ ({GV / ref}). "
            "Name an existing <site>_hosts directory in .groupvars-parity.yaml "
            "or with --scaffold-from.")
    return ref, None


def check_file_inventory(group):
    """[30/32] Skeleton-as-file-inventory: a new env's config surface is the
    REFERENCE env's FILE SET, not one vars.yml. Every reference file missing
    from this env must be present OR declared as an intentional exclusion (with
    a reason) in .groupvars-parity.yaml. A missing-and-undeclared file FAILS —
    that's how 22 services were declared in isitup_services yet silently never
    deployed (their per-service group_vars file simply didn't exist)."""
    manifest = GV / ".groupvars-parity.yaml"
    if not manifest.is_file():
        return  # no parity manifest → skip (not every env opts in)
    spec = yaml.safe_load(manifest.read_text()) or {}
    ref, why = resolve_reference(spec)
    if why:
        warn(f"file-inventory check skipped: {why}")
        return
    excl = (spec.get("exclusions", {}) or {}).get(group, {}) or {}
    file_excl, group_class = split_exclusions(excl)
    ref_files = {p.name for p in (GV / ref).glob("*.yml")}
    env_files = {p.name for p in (GV / group).glob("*.yml")}
    missing = sorted(ref_files - env_files - {"pins.yml", "vars.yml"})
    by_class = []
    for f in missing:
        if f in file_excl:
            continue
        if group_class:
            # Scoped by construction: `missing` is derived from ref_files, so a
            # class can only ever waive files the reference actually has. It can
            # never invent an exemption for a file nobody ships.
            by_class.append(f)
            continue
        fail(f"[file-inventory] {group} lacks {f} (present in reference {ref}) and "
             f"it is NOT a declared exclusion — port it, or add it to "
             f".groupvars-parity.yaml exclusions.{group} with a reason (row 30/32)")
    if by_class:
        warn(f"[file-inventory] {group}: {len(by_class)} reference file(s) waived by "
             f"{GROUP_CLASS_KEY} — {group_class} · waived: {', '.join(by_class)}")
    # hygiene: an exclusion for a file the reference no longer has is dead.
    # GROUP_CLASS_KEY is not a filename and is exempt from this by definition.
    for f in file_excl:
        if f not in ref_files:
            warn(f"[file-inventory] exclusion {group}.{f} names a file no longer "
                 f"in reference {ref} — remove the stale exclusion")


def check_capability_toggles(group):
    """[F-437] The file-inventory rule above catches a missing FILE. It cannot
    catch a capability the reference turns on with a bare `<thing>_enabled: true`
    INSIDE a file both envs have — vars.yml. Nothing is missing, so nothing is
    reported, and the two envs quietly diverge.

    A sandbox can set `herd_enabled` and `netview_enabled` and carry
    `compliance` / `netview-response` isitup rows that its production sibling
    lacks, while this script and the file-inventory check report OK, because no
    FILE is absent. Surfaces then differ between a sandbox and its production
    sibling with no record saying whether that is a decision.

    OPT-IN, deliberately. Enrolment is per-group under `capability_parity` so
    turning this on cannot red-line every env at once — a fleet-wide flag day
    would be reverted rather than answered. An enrolled group must, for each
    listed toggle, either SET it or carry a reason. Same shape as the file
    rule: absence-as-data, every absence gets a home."""
    manifest = GV / ".groupvars-parity.yaml"
    if not manifest.is_file():
        return
    spec = yaml.safe_load(manifest.read_text()) or {}
    enrolled = (spec.get("capability_parity", {}) or {}).get(group)
    if not enrolled:
        return  # group not enrolled — silent by design
    ref, why = resolve_reference(spec)
    if why:
        warn(f"capability-parity check skipped: {why}")
        return
    ref_vals, _ = load_group(ref)
    env_vals, _ = load_group(group)
    declared = enrolled if isinstance(enrolled, dict) else {}
    # Every *_enabled the REFERENCE turns on is in scope. Deriving the scope
    # from the reference rather than from a hand-list means a capability added
    # to the reference later cannot slip past by nobody remembering to list it.
    ref_on = sorted(k for k, v in ref_vals.items()
                    if k.endswith("_enabled") and v is True)
    for key in ref_on:
        if key in env_vals:
            continue  # target has made a call, either way
        if key in declared:
            continue  # absence has a recorded reason
        fail(f"[capability-parity] {group} does not set {key}, which reference "
             f"{ref} enables — set it, or record why under "
             f"capability_parity.{group} in .groupvars-parity.yaml")
    for key in declared:
        if key not in ref_on:
            warn(f"[capability-parity] {group}.{key} names a toggle reference "
                 f"{ref} no longer enables — remove the stale entry")


def check_rescue_nodes(values, group):
    """[ADR 1183] A site says what rescue nodes it has, or says why it has none.

    Two classes exist for the worst day a site has — BREAKGLASS (ADR 1114), the
    way in when the mesh and the control plane are both down, and NETWORK-JUMP
    (ADR 1131), the reach to switches, routers and out-of-band gear management.
    Neither was part of the standup path, and the result was a break-glass node
    with no config scope on its own goatnet, a jump class with zero peers
    anywhere, and a 47-hour outage of a rescue node that nothing graded.

    The check is on the ANSWER, not on the node: "none, because the switch is in
    the same cabinet as the operator" is a fine answer for a single-rack site.
    Silence is not, because silence and oversight are the same thing here."""
    for flag, reason_key, adr, what in (
        ("has_breakglass_node", "breakglass_absent_reason", "ADR 1114",
         "a break-glass node — the way in when the mesh and the control plane are both down"),
        ("has_network_jump_node", "network_jump_absent_reason", "ADR 1131",
         "a network-jump host — reach to the switch, the router and out-of-band gear management"),
    ):
        if flag not in values:
            fail(f"{flag} undefined — this site must declare whether it has "
                 f"{what} ({adr}). Set `{flag}: true` with the node in inventory "
                 f"group [{group.replace('_hosts', '')}_{flag[4:-5]}_hosts], or "
                 f"`{flag}: false` with `{reason_key}: <why>` (ADR 1183)")
            continue
        if values[flag] is True:
            continue
        reason = str(values.get(reason_key, "") or "").strip()
        if not reason:
            fail(f"{flag} is false with no {reason_key} — record in one sentence "
                 f"what covers this site instead, or what is on order (ADR 1183)")


def _mgmt_hosts_children(inv):
    """The group names listed under [mgmt_hosts:children] in `inv`, or None
    when the file or the section is absent."""
    if not inv.is_file():
        return None
    out, inside, seen = set(), False, False
    for line in inv.read_text().splitlines():
        st = line.split("#", 1)[0].split(";", 1)[0].strip()
        if st.startswith("["):
            inside = st == "[mgmt_hosts:children]"
            seen = seen or inside
            continue
        if inside and st:
            out.add(st.split()[0])
    return out if seen else None


def check_mgmt_hosts_membership(group, v):
    """[Spock discovery] Every inter-site Spock role finds its peers by walking
    groups['mgmt_hosts'] (roles/spock-mesh-peering). A management-stack group
    that is not a child of [mgmt_hosts:children] in inventory/mgmt-hosts.ini is
    invisible to that walk: a founder missing from it leaves every joiner with
    zero peers. Checked for this group and the groups it federates with (its
    join_founder_site and inter_site_spock_peers), each that runs the
    management stack (mgmt_stack_profile: full, or has_mgmt_api: true)."""
    inv = REPO / "ops" / "ansible" / "inventory" / "mgmt-hosts.ini"
    names = [group]
    founder = str(v.get("join_founder_site") or "").strip()
    peers = v.get("inter_site_spock_peers") or []
    for site in ([founder] if founder else []) + [str(p) for p in peers if isinstance(p, str)]:
        if "{{" in site:
            continue
        g = site.replace("-", "_") + "_hosts"
        if g not in names:
            names.append(g)
    needs = []
    for g in names:
        gv = v if g == group else (load_group(g)[0] if (GV / g).is_dir() else None)
        if gv is None:
            continue
        if str(gv.get("mgmt_stack_profile", "")).strip() == "full" or gv.get("has_mgmt_api") is True:
            needs.append(g)
    if not needs:
        return
    children = _mgmt_hosts_children(inv)
    if children is None:
        fail(f"[mgmt_hosts] inventory/mgmt-hosts.ini has no "
             f"[mgmt_hosts:children] section — Spock peer discovery walks groups['mgmt_hosts'] "
             f"and would find none of {', '.join(needs)}. Add the section listing every "
             f"management-stack group.")
        return
    for g in needs:
        if g not in children:
            fail(f"[mgmt_hosts] {g} runs the management stack but is not a child of "
                 f"[mgmt_hosts:children] in inventory/mgmt-hosts.ini — Spock peer discovery "
                 f"walks groups['mgmt_hosts'], so every site federating with it sees zero "
                 f"peers. Add the line `{g}` under [mgmt_hosts:children].")


def load_group(group):
    d = GV / group
    if not d.is_dir():
        sys.exit(f"standup-config-preflight: no group_vars dir {d}")
    merged, raw = {}, ""
    for f in sorted(d.glob("*.yml")):
        text = f.read_text()
        raw += text
        try:
            data = yaml.safe_load(text) or {}
        except yaml.YAMLError as e:
            sys.exit(f"unparseable {f}: {e}")
        if isinstance(data, dict):
            merged.update(data)
    return merged, raw


def scaffold_plan(group, ref_override=None):
    """--scaffold-plan: what config surface does this goatnet still owe?

    The rest of this script refuses when group_vars/<group>/ is absent, which is
    correct before a play runs and useless to the person who has not authored the
    surface yet — the state EVERY new goatnet starts in. This mode answers the
    prior question: given the reference env's file set, which files does this env
    have, which are missing, and which absences are already declared. Offline,
    read-only, and safe to run against a group that does not exist at all.
    """
    manifest = GV / ".groupvars-parity.yaml"
    if not manifest.is_file():
        sys.exit(f"scaffold-plan: no {manifest} — nothing defines 'complete'")
    spec = yaml.safe_load(manifest.read_text()) or {}
    ref, why = resolve_reference(spec, ref_override)
    if why:
        sys.exit(f"scaffold-plan: {why}")
    excl = (spec.get("exclusions", {}) or {}).get(group, {}) or {}
    file_excl, group_class = split_exclusions(excl)
    ref_files = sorted(q.name for q in (GV / ref).glob("*.yml"))
    have = {q.name for q in (GV / group).glob("*.yml")} if (GV / group).is_dir() else set()

    print(f"scaffold-plan: {group}  (reference: {ref})")
    if not (GV / group).is_dir():
        print(f"  group_vars/{group}/ does not exist yet — every file below is owed.")
    if group_class:
        print(f"  class: {group_class}")
        print("  Files below marked by-class are waived by that declaration, not ported.")
    print()
    owed, declared, classed = [], [], []
    for f in ref_files:
        note = ""
        if f in have:
            mark = "have"
        elif f in file_excl:
            mark = "excluded"
            declared.append(f)
            note = f"   ({file_excl[f]})"
        elif group_class:
            mark = "by-class"
            classed.append(f)
        else:
            mark = "MISSING"
            owed.append(f)
        print(f"  {mark:9s} {f}{note}")
    print()
    print(f"  {len(have & set(ref_files))} present · {len(owed)} missing · "
          f"{len(declared)} declared-absent · {len(classed)} waived-by-class · "
          f"{len(ref_files)} in reference")
    if owed:
        print()
        print("  Each MISSING file is a decision, not a chore: port it with every")
        print(f"  value re-threaded for {group}, or declare the absence with a reason")
        print(f"  in {manifest.relative_to(REPO)} under exclusions.{group}.")
        print("  Copying a reference file without re-threading mints the WRONG")
        print("  goatnet's identity — URI SANs and anchors are freeze-critical")
        print("  (Class A, docs/design/instance-variability-model.md §1).")
    return 1 if owed else 0



# --- scaffold ---------------------------------------------------------------
# Values a scaffold may derive mechanically, because they are a pure function of
# the new site's identity. Everything else is either a secret or a decision, and
# both become TBD_ sentinels rather than a copied value.
#
# WHY NOT JUST COPY: copying a reference file without re-threading mints the
# WRONG goatnet's identity — URI SANs and storage anchors are freeze-critical
# (Class A, docs/design/instance-variability-model.md §1). Two real environments
# differ in over a hundred keys, so
# "copy and edit what looks wrong" misses most of them silently.

# key suffix -> how to fill it. Anything not listed becomes TBD_ if its value
# looks env-specific (see _looks_env_specific).
_DERIVE_SUFFIX = ("_site", "_site_id")

# A literal public IPv4 in the reference names one of ITS hosts (a relay, an
# ingress) even though it names no environment, so a name check cannot see it.
# Another goatnet's relay IPs can survive this way in a scaffolded enrolment-name
# record. Every such literal becomes TBD_.
_PUBLIC_V4 = re.compile(r"(?<![\d.])((?:\d{1,3}\.){3}\d{1,3})(?![\d.])")


def _names_reference_address(val):
    import ipaddress
    for ip in _PUBLIC_V4.findall(val or ""):
        try:
            a = ipaddress.ip_address(ip)
        except ValueError:
            continue
        if a.is_global and a not in ipaddress.ip_network("100.64.0.0/10"):
            return True
    return False


# A value carrying any of these is a secret or a pin. Never carried across.
# Goatnet names as a human writes them in prose. Derived stems miss these:
# the group dir is `nova_hosts`, but nobody writes "nova" in a comment.
def _declared_goatnet_names():
    """`goatnet_prose_names` in ops/goatnets/cross-env.yaml; none on an instance without it."""
    path = Path(__file__).resolve().parents[1] / "goatnets" / "cross-env.yaml"
    try:
        doc = yaml.safe_load(path.read_text()) or {}
    except OSError:
        return ()
    return tuple(doc.get("goatnet_prose_names") or ())


_KNOWN_GOATNETS = _declared_goatnet_names()

_NEVER_CARRY = (
    "_secret", "_password", "_api_key", "_access_key", "_secret_key",
    "_token", "_image", "_digest", "_passphrase",
)


def _names_other_goatnet(site, text):
    """Does this prose name a goatnet that is not this one?

    Comments ride across a copy untouched and then read as statements about the
    new site. A reference vars.yml can open with lines describing another site
    (wrong zone, wrong hardware, wrong runbook) because it was copied and the
    prose was never re-threaded.
    """
    low = text.lower()
    site_l = site.lower()
    for name in _KNOWN_GOATNETS:
        if name in site_l or site_l in name:
            continue
        if re.search(rf"\b{re.escape(name)}\b", low):
            return True
    return False


def _known_env_stems():
    """Every goatnet name the repo knows, so a scaffold can spot one anywhere.

    Checking only the reference env is not enough: a reference file can carry a
    comment about a THIRD environment ("no obs host at that site today"), which
    then rides into the new site's config looking like a statement about it.
    """
    stems = set()
    for d in GV.glob("*_hosts"):
        stem = d.name.replace("_hosts", "")
        stems.add(stem)
        stems.add(stem.replace("_", "-"))
    return {s for s in stems if len(s) > 2}


def _looks_env_specific(ref_env, site, val):
    """Does this literal name SOME environment rather than state a fact?"""
    if not isinstance(val, str) or not val.strip():
        return False
    low = val.lower()
    site_stem = site.replace("-", "_")
    for stem in _known_env_stems():
        if stem in (site, site_stem):
            continue
        if stem in low:
            return True
    return False


def scaffold(group, site, ref_override=None, force=False):
    """--scaffold: write the owed config surface as reviewable TBD_-marked files.

    --scaffold-plan answers "what do I owe"; this answers "give me something to
    edit". It never invents an identity: a value that is a secret, an image pin,
    or that names the reference environment becomes a TBD_ sentinel, which the
    normal preflight run already fails closed on ("TBD sentinel still present").
    So a scaffolded surface cannot be deployed until a human has been through it.
    """
    manifest = GV / ".groupvars-parity.yaml"
    if not manifest.is_file():
        sys.exit(f"scaffold: no {manifest} — nothing defines 'complete'")
    spec = yaml.safe_load(manifest.read_text()) or {}
    ref, why = resolve_reference(spec, ref_override)
    if why:
        sys.exit(f"scaffold: {why}")
    excl = (spec.get("exclusions", {}) or {}).get(group, {}) or {}
    file_excl, group_class = split_exclusions(excl)

    dest = GV / group
    dest.mkdir(parents=True, exist_ok=True)
    upper = site.replace("-", "_").upper()

    written, skipped, tbds = [], [], 0
    for src in sorted((GV / ref).glob("*.yml")):
        if src.name in file_excl or group_class:
            skipped.append(src.name)
            continue
        out = dest / src.name
        if out.exists() and not force:
            skipped.append(src.name)
            continue
        lines, n, stale = [], 0, []
        parent = "VALUE"
        for line in src.read_text(errors="ignore").split("\n"):
            m = re.match(r"^([a-z0-9_]+):[ \t]*(.*)$", line)
            if not m:
                if line.lstrip().startswith("#"):
                    if _names_other_goatnet(site, line):
                        stale.append(line.lstrip()[:88])
                    lines.append(line)
                    continue
                # A list item or an indented value is config too. Carrying these
                # across unchecked is how a new goatnet ends up using a sibling's
                # STUN servers: `stun_hosts:` matches above, its list items do not,
                # and they are copied verbatim.
                code = line.split(" #", 1)[0]
                item = re.match(r"^(\s*)-\s+(.*)$", code)
                nested = re.match(r"^(\s+)([A-Za-z0-9_]+):[ \t]*(.+)$", code)
                val = item.group(2) if item else (nested.group(3) if nested else "")
                if val and (_looks_env_specific(ref, site, val) or _names_reference_address(val)):
                    if item:
                        lines.append(f"{item.group(1)}- TBD_{parent.upper()}   # was: {val.strip()[:48]}")
                    else:
                        lines.append(f"{nested.group(1)}{nested.group(2)}: TBD_{parent.upper()}_{nested.group(2).upper()}   # was: {val.strip()[:48]}")
                    n += 1
                else:
                    lines.append(line)
                continue
            parent = m.group(1)
            key, val = m.group(1), m.group(2).strip()
            if any(key.endswith(x) for x in _DERIVE_SUFFIX):
                lines.append(f"{key}: {site}")
            elif any(x in key for x in _NEVER_CARRY) and val and not val.startswith("#"):
                lines.append(f"{key}: TBD_{key.upper()}")
                n += 1
            elif _looks_env_specific(ref, site, val) or _names_reference_address(val):
                lines.append(f"{key}: TBD_{key.upper()}   # was: {val[:48]}")
                n += 1
            else:
                lines.append(line)
        hdr = (
            f"# SCAFFOLDED from group_vars/{ref}/{src.name} for {group}.\n"
            f"# Every TBD_ below is a decision this scaffold refused to guess: a secret,\n"
            f"# an image pin, or a value that named {ref} rather than stating a fact.\n"
            f"# `standup-config-preflight.py {group}` fails closed while any remain.\n"
            f"# Env-var prefix for this site is {upper}_.\n"
        )
        if stale:
            hdr += (
                f"#\n# {len(stale)} comment line(s) below name another goatnet and were\n"
                f"# carried across verbatim. Re-thread or delete them — prose that names\n"
                f"# the wrong environment reads as a statement about this one:\n"
                + "".join(f"#   {t}\n" for t in stale[:6])
                + ("#   ...\n" if len(stale) > 6 else "")
            )
        out.write_text(hdr + "\n".join(lines))
        written.append((src.name, n, len(stale)))
        tbds += n

    print(f"scaffold: {group}  (from reference: {ref}, site id: {site})")
    for f, n, st in written:
        bits = []
        if n: bits.append(f"{n} TBD_")
        if st: bits.append(f"{st} stale comment(s)")
        print(f"  wrote     {f}" + (f"   [{', '.join(bits)}]" if bits else ""))
    for f in skipped:
        print(f"  skipped   {f}   (already present, or excluded/by-class)")
    print()
    stale_total = sum(st for _, _, st in written)
    print(f"  {len(written)} written · {len(skipped)} skipped · {tbds} TBD_ sentinels"
          + (f" · {stale_total} stale comment line(s) flagged" if stale_total else ""))
    print()
    print("  NOTHING HERE IS DEPLOYABLE YET, by design. Each TBD_ is a decision the")
    print("  scaffold refused to guess. Work through them, then run:")
    print(f"    ops/scripts/standup-config-preflight.py {group}")
    print("  which fails closed on any TBD_ that remains.")
    return 1 if tbds else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("group", help="mgmt host group, e.g. zagros_prod_hosts")
    ap.add_argument("--obs", help="obs host group, e.g. zagros_prod_obs_hosts")
    ap.add_argument("--scaffold", metavar="SITE_ID",
                    help="write the owed config surface, TBD_-marking every value "
                         "that is a secret, a pin, or names the reference env")
    ap.add_argument("--scaffold-from", metavar="GROUP",
                    help="reference group for --scaffold and --scaffold-plan "
                         "(default: the parity manifest's `reference`)")
    ap.add_argument("--force", action="store_true",
                    help="overwrite files that already exist")
    ap.add_argument("--scaffold-plan", action="store_true",
                    help="list the config files this env still owes relative to "
                         "the reference env, then exit; works before "
                         "group_vars/<group>/ exists")
    args = ap.parse_args()

    if args.scaffold:
        sys.exit(scaffold(args.group, args.scaffold, args.scaffold_from, args.force))
    if args.scaffold_plan:
        sys.exit(scaffold_plan(args.group, args.scaffold_from))

    v, raw = load_group(args.group)

    # [30/32] the config surface is a FILE SET, not one vars.yml
    check_file_inventory(args.group)

    # [Spock discovery] management-stack groups are children of mgmt_hosts
    check_mgmt_hosts_membership(args.group, v)

    # [ADR 1183] the two rescue classes are declared, or their absence is
    check_rescue_nodes(v, args.group)

    # [P2] no value names another goatnet's domain (list items included) unless
    # the line says `# cross-goatnet: <reason>`. Same check CI runs over every
    # group; running it here fails the standup before a deploy, not after.
    fg = REPO / "ops" / "ci" / "check-no-foreign-goatnet-config.py"
    if fg.is_file():
        import importlib.util
        spec = importlib.util.spec_from_file_location("foreign_goatnet", fg)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        for path, n, group, mine, other, code in mod.scan(GV):
            if group == args.group or (args.obs and group == args.obs):
                fail(f"{path}:{n} names {other}, another goatnet's domain "
                     f"(this goatnet is {mine}): {code[:80]}")
        # [P2b] the same, for relay ADDRESSES: a bare IP names no domain, so the
        # check above cannot see a sibling's relays answering this goatnet's enrolment name.
        if hasattr(mod, "scan_relay_ips"):
            mine_dirs = {args.group} | ({args.obs} if args.obs else set())
            for path, n, where, mine, other, what in mod.scan_relay_ips(REPO):
                if where in mine_dirs:
                    fail(f"{path}{':' + str(n) if n else ''}: {what} (this goatnet is {mine}; "
                         f"use its own relays from ops/relay/health/relays*.conf)")

    # [P1] TBD_ sentinels — the runbook's own first-boot gate, surfaced early.
    for m in sorted(set(re.findall(r"TBD_[A-Z0-9_]+", raw))):
        fail(f"TBD sentinel still present: {m}")

    # [16] sidecar owns 127.0.0.1:8080 — every goatnet must override.
    if "mgmt_container_host_port" not in v:
        fail("mgmt_container_host_port undefined — first compose up will hit "
             "address-already-in-use against the ntp-admit-sidecar (row 16)")

    # [22] cleartext signal: peers register but stay Signal-disconnected.
    if str(v.get("signal_proto", "http")).lower() != "https":
        fail("signal_proto is not 'https' — peers dial cleartext h2c and the "
             "TLS-scoped signal router never matches (row 22)")

    # [21] empty netbird OIDC auth fields render clean, fail at first login.
    for key in ("auth_issuer", "auth_audience", "auth_oidc_config_endpoint"):
        val = str(v.get(key, "") or "")
        if not val.strip():
            fail(f"{key} empty/undefined — management.json renders with empty "
                 "auth fields and rejects every dashboard token (row 21)")
    issuer = str(v.get("auth_issuer", "") or "")
    if issuer and not issuer.endswith("/"):
        fail("auth_issuer lacks the trailing slash the netbird OIDC validator "
             "requires (zagros_hosts vars.yml comment block)")

    # [20] Ory secret env mappings present (spot the family, not every key).
    ory_keys = [k for k in v if re.match(r"(kratos|hydra|ory)_", k)]
    if not ory_keys:
        fail("no kratos_/hydra_/ory_* keys — Ory secret env mappings missing "
             "from this goatnet's group_vars (row 20)")

    # [25] pin/flag coupling: zenoh-pqc-fips image REQUIRES fips_variant true,
    # else every storage dies behind a healthy container.
    zimg = ""
    for k, val in v.items():
        if re.search(r"zenoh.*image$", k) and isinstance(val, str):
            zimg = val
            break
    fips_flag = bool(v.get("goat_zenoh_fips_variant", False))
    if "pqc-fips" in zimg and not fips_flag:
        fail("zenoh image pin is the pqc-fips variant but "
             "goat_zenoh_fips_variant is not true — storages fail to "
             "spawn while the container reports healthy (row 25, F-260 class)")
    if fips_flag and zimg and "fips" not in zimg:
        fail("goat_zenoh_fips_variant true but the pinned zenoh image is "
             "not a fips variant — the paired pin moved without its partner")

    # [54-class] six Class-A cert-identity strings (CI also gates these once
    # portal_image renders; assert here so it fails BEFORE generate-pins).
    # F-394 — decide "does the portal render here?" from the PARSED keys, not
    # from a substring of the concatenated raw file text. `raw` includes
    # comments, and generate-pins.py writes the words "portal_image" into the
    # prose explaining why a group has NO portal:
    #
    #   # implying a service that isn't there, rendering portal_image into
    #
    # so a joiner that correctly declares the portal absent would be told its
    # cert identity is undefined — three FAILs describing the opposite of the
    # truth.
    # Same signal set as ops/ci/check-portal-cert-identity-override.sh, and the
    # Keeping the two in step is the point — the preflight exists to fail before
    # CI does, never to disagree with it.
    portal_renders_here = any(
        k == "portal_image" or k.startswith("portal_cert_sign_")
        or k == "portal_identity_model" for k in v
    )
    # A named LAN site is a standalone LAN instance that keeps the legacy identity; same
    # allowlist as ops/ci/check-portal-cert-identity-override.sh.
    nova_lan = args.group == "nova_lan_hosts"
    for key in ("portal_cert_sign_uri_prefix", "portal_cert_sign_cn_suffix",
                "portal_cert_sign_dns_suffix"):
        if portal_renders_here and not nova_lan and key not in v:
            fail(f"{key} undefined while portal renders into this env — certs "
                 "would mint with the nova identity (turnkey doc Class-A)")

    # [19] capability flags that need their substrate.
    if bool(v.get("snitch_enabled", False)):
        col = str(v.get("snitch_collector_url", "") or "")
        if (not col.strip()) or col.startswith("TBD_"):
            fail("snitch_enabled true but snitch_collector_url empty/TBD — "
                 "probe-key extract fails on fresh hosts (row 19)")

    # [27] obs leg: per-Orin facts must exist in the OBS group too, because
    # site-bootstrap's F-093 assert never runs there.
    if args.obs:
        ov, _oraw = load_group(args.obs)
        if "luks_volumes_mgmt_host" not in ov:
            fail(f"{args.obs}: luks_volumes_mgmt_host undefined — "
                 "mgmt-host-luks.yml aborts (row 27)")
        if "docker_engine_data_root" not in ov:
            fail(f"{args.obs}: docker_engine_data_root undefined — daemon.json "
                 "renders with no data-root and Docker writes to unencrypted "
                 "eMMC while the LUKS volume sits empty (row 27, silent)")

    check_capability_toggles(args.group)

    print(f"standup-config-preflight: {args.group}"
          + (f" + {args.obs}" if args.obs else ""))
    for w in WARNS:
        print(f"  WARN: {w}")
    if FAILS:
        for f_ in FAILS:
            print(f"  FAIL: {f_}")
        print(f"{len(FAILS)} check(s) failed — fix group_vars before the "
              "first play; each failure above cost a live retry on 2026-08-04.")
        return 1
    print("  OK: all standup-config checks pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
