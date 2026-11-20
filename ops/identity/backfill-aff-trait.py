#!/usr/bin/env python3
"""Put the `aff` (affiliation) on identities minted before Phase D was on.

Phase D of the identity grammar (ADR 1043) carries WHO an identity belongs to
as `aff = {id, kind[, nation]}`, and the patched consent UI forwards it
into the id_token as the `aff` claim. Both mint paths gate it behind a flag
that stayed off on zagros after the 2026-07-13 live fire, so a read of
the zagros Hydra on 2026-09-06 found no id_token carrying the claim across 1096
consent sessions, and two identities holding an EMPTY aff. A relying party
that authorizes on `aff.id` (ADR 1043 Phase D) sees no affiliation in a
token without it. The flag is on for zagros from 2026-09-07; this script
closes the gap for the identities minted before it.

WHERE IT IS WRITTEN. `aff` is an authorization field, so it lives in
admin-managed `metadata_public`, never in self-editable traits (F-572 / ADR
1188). Reads follow every other reader (GHSA-9v3v): only `metadata_public`
counts, migrated or not. A traits.org or traits.aff is a value the holder could
have written through the settings flow, so it never selects the org whose aff
is written here. Run the authz backfill (backfill-authz-traits-to-metadata.py,
with its --trusted record) first; an identity it has not moved has no
org.short_tag here and is skipped and listed (or named with --assign). The
write adds the whole `metadata_public` object with its other keys kept, so it
works on an identity that has none yet, and it never seeds `groups`: that is
the authz backfill's marker.

WHERE THE VALUE COMES FROM. `org.short_tag`: both mint paths and the
org backfill set it from the manifest's `affiliation.id`, the shared unit
slug. It is resolved in the committed org catalog (ops/identity/orgs/<env>.yaml,
the console org store's projection, ADR 1112) by id or alias, and kind and
nation come from the catalog row, the same rule the console's identity page
applies. An identity whose short_tag is absent or unknown to the catalog is
skipped and listed; --assign EMAIL=ORG_ID names the org for it by hand, and
an org the catalog does not hold is refused before anything is read or
written, under --verify as under --apply. An existing non-empty aff is NEVER
changed (the trait is forward-only): one that disagrees with the catalog is
reported.

Runs ON the mgmt host (Kratos admin API on 127.0.0.1:4434 by default).
YAML catalogs use PyYAML, provided by the management host's python3-yaml
package. Copy this script and orgs/<env>.yaml, keeping that layout, and pass
--env <env>. Without PyYAML, use a JSON export: --catalog FILE.json, or
orgs/<env>.json, which --env reads in preference to the YAML. The catalog
read is printed first.
Dry-run by default; --apply writes.

Examples:
  python3 backfill-aff-trait.py --env zagros --verify   # counts, exit 1 while a fillable identity lacks aff.id
  python3 backfill-aff-trait.py --env zagros            # dry-run: show planned changes
  python3 backfill-aff-trait.py --env zagros --only a@b.test --apply   # trial on one identity
  python3 backfill-aff-trait.py --env zagros --assign a@b.test=org11x --apply
  python3 backfill-aff-trait.py --env zagros --apply
"""

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_ADMIN = "http://127.0.0.1:4434"
PAGE_SIZE = 500
# The identity schema's aff constraints (roles/ory/templates/ory/identity-schema.json.j2).
ID_PATTERN = re.compile(r"^[a-z0-9-]{1,64}$")
KINDS = ("vendor", "organic", "partner", "civil")
NATION_MAX = 64
HERE = os.path.dirname(os.path.abspath(__file__))


def _req(method, url, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Accept", "application/json")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    with urllib.request.urlopen(req, timeout=30) as resp:
        raw = resp.read()
        return (json.loads(raw) if raw else None), dict(resp.headers)


def _next_link(headers):
    """The rel="next" path from Kratos's keyset-paginated Link header, or None."""
    link = headers.get("Link") or headers.get("link") or ""
    for part in link.split(","):
        m = re.search(r"<([^>]+)>\s*;\s*rel=\"?next\"?", part)
        if m:
            return m.group(1)
    return None


def list_identities(admin):
    """Every identity, following the Link rel="next" until there is none.

    The Link header is the only end-of-list signal: a page shorter than
    PAGE_SIZE can still have a next page, and stopping on one dropped every
    identity after it, from the backfill and from --verify alike. A page token
    served twice would loop, so it is an error, never a quiet stop."""
    out = []
    url = f"{admin}/admin/identities?page_size={PAGE_SIZE}"
    seen_tokens = set()
    while url:
        batch, headers = _req("GET", url)
        out.extend(batch or [])
        nxt = _next_link(headers)
        if not nxt:
            return out
        url = urllib.parse.urljoin(admin, nxt)
        tok = urllib.parse.parse_qs(urllib.parse.urlparse(url).query).get("page_token", [""])[0]
        if tok in seen_tokens:
            raise ValueError(f"Kratos served page_token {tok!r} twice; the identity list is incomplete")
        seen_tokens.add(tok)
    return out


def catalog_path(catalog, env):
    """--catalog as given; for --env, orgs/<env>.json when present, else orgs/<env>.yaml."""
    if catalog or not env:
        return catalog
    base = os.path.join(HERE, "orgs", env)
    return base + ".json" if os.path.exists(base + ".json") else base + ".yaml"


def load_catalog(path):
    """Index YAML or JSON org rows by id and alias; JSON needs no PyYAML."""
    try:
        with open(path, encoding="utf-8") as source:
            text = source.read()
    except (OSError, UnicodeError) as exc:
        raise SystemExit(f"{path}: cannot read catalog: {exc}") from exc
    if path.endswith((".yaml", ".yml")):
        try:
            import yaml
        except ImportError as exc:
            env = os.path.splitext(os.path.basename(path))[0]
            raise SystemExit(f"{path}: PyYAML is unavailable; pass --catalog orgs/{env}.json") from exc
        try:
            document = yaml.safe_load(text)
        except Exception as exc:  # SafeLoader constructors also raise built-in exceptions.
            raise SystemExit(f"{path}: invalid YAML catalog: {exc}") from exc
    else:
        try:
            document = json.loads(text)
        except ValueError as exc:
            raise SystemExit(f"{path}: invalid JSON catalog: {exc}") from exc
    if not isinstance(document, dict) or not isinstance(document.get("orgs"), list):
        raise SystemExit(f"{path}: expected a mapping with an orgs list")
    by_slug = {}
    for row in document["orgs"]:
        if not isinstance(row, dict):
            raise SystemExit(f"{path}: each org must be a mapping")
        aliases = row.get("aliases")
        if aliases is not None and not isinstance(aliases, list):
            raise SystemExit(f"{path}: org aliases must be a list")
        org_id = str(row.get("id") or "").strip()
        if not org_id:
            continue
        by_slug[org_id] = row
        for alias in aliases or []:
            by_slug.setdefault(str(alias).strip(), row)
    if not by_slug:
        raise SystemExit(f"{path}: no orgs in the catalog")
    return by_slug


def aff_for(row):
    """The aff value the schema accepts for one catalog row, or (None, why)."""
    org_id = str(row.get("id") or "")
    kind = str(row.get("kind") or "").strip()
    nation = str(row.get("nation") or "").strip()
    status = str(row.get("status") or "").strip()
    if status != "active":
        return None, f"catalog org {org_id!r} is {status or 'unset'}, not active"
    if not ID_PATTERN.match(org_id):
        return None, f"catalog org id {org_id!r} does not match the identity schema"
    if kind not in KINDS:
        return None, f"catalog kind {kind!r} for {org_id!r} is not one the identity schema accepts"
    if len(nation) > NATION_MAX:
        return None, f"catalog nation for {org_id!r} is longer than {NATION_MAX}"
    aff = {"id": org_id, "kind": kind}
    if nation:
        aff["nation"] = nation
    return aff, None


def authz_val(identity, key):
    """An authorization field (groups, org, aff) as every reader resolves it:
    from admin-managed metadata_public only. Traits are self-editable and are
    never read, migrated or not (GHSA-9v3v), the same rule as the
    id-admin-console's Identity.authzVal and consent patches 0001/0004."""
    return (identity.get("metadata_public") or {}).get(key)


def current_aff_id(identity):
    if not isinstance(identity, dict):
        return None
    aff = authz_val(identity, "aff")
    if isinstance(aff, dict):
        return str(aff.get("id") or "").strip() or None
    return None


def parse_assignments(values):
    out = {}
    for value in values or []:
        email, sep, org = value.partition("=")
        if not sep or not email.strip() or not org.strip():
            raise SystemExit(f"--assign wants EMAIL=ORG_ID, got {value!r}")
        out[email.strip().lower()] = org.strip()
    return out


def unknown_assignments(assign, catalog):
    """The --assign entries that name an org the catalog does not hold."""
    return [f"{email}={org}" for email, org in assign.items() if org not in catalog]


def plan(identities, catalog, only=None, assign=None):
    """Return (actions, skipped). An action puts one aff on one identity."""
    actions, skipped = [], []
    wanted = only.strip().lower() if only else None
    assign = assign or {}
    for identity in identities:
        traits = identity.get("traits") or {}
        email = str(traits.get("email") or "").strip().lower()
        ident = {"id": identity.get("id"), "email": email or "?"}
        if wanted and email != wanted:
            continue
        tag = assign.get(email) or str((authz_val(identity, "org") or {}).get("short_tag") or "").strip()
        source = "assign" if email in assign else "org.short_tag"
        row = catalog.get(tag) if tag else None
        have = current_aff_id(identity)
        if have:
            why = f"already carries aff {have!r}"
            if row is not None and str(row.get("id") or "") != have:
                why += f"; the catalog resolves {tag!r} to {row['id']!r}, forward-only, left alone"
            skipped.append({**ident, "why": why})
            continue
        if not tag:
            skipped.append({**ident, "why": "no org.short_tag on the identity (use --assign)"})
            continue
        if row is None:
            skipped.append({**ident, "why": f"{source} {tag!r} is not in the catalog (use --assign)"})
            continue
        aff, why = aff_for(row)
        if aff is None:
            skipped.append({**ident, "why": why})
            continue
        meta = dict(identity.get("metadata_public") or {})
        meta["aff"] = aff
        actions.append({**ident, "aff": aff, "metadata_public": meta, "source": source, "tag": tag})
    return actions, skipped


def patch_aff(admin, action):
    """JSON Patch add on /metadata_public: the identity's metadata_public with
    aff set and every other key kept. Writing the whole object is safe because
    it keeps every sibling key; Kratos would also accept a child-path add (it
    creates a missing parent).
    An empty traits.aff (two zagros identities carry one) is left for the authz
    backfill to remove; readers already prefer the metadata_public value.
    Returns the identity as Kratos stored it."""
    body = [{"op": "add", "path": "/metadata_public", "value": action["metadata_public"]}]
    url = f"{admin}/admin/identities/{urllib.parse.quote(action['id'])}"
    result, _ = _req("PATCH", url, body)
    return result


def apply_one(admin, action):
    """Write one aff and check it on the PATCH response; True when it took.

    The response is the stored identity, so it answers whether aff.id landed
    without a second GET per identity."""
    try:
        stored = patch_aff(admin, action)
    except (urllib.error.URLError, OSError, ValueError) as exc:
        print(f"error  {action['email']:<44} {exc}", file=sys.stderr)
        return False
    got = current_aff_id(stored)
    if got != action["aff"]["id"]:
        print(f"error  {action['email']:<44} patched, but the stored identity carries aff.id {got!r}",
              file=sys.stderr)
        return False
    return True


def _label(identity):
    return str((identity.get("traits") or {}).get("email") or identity.get("id"))


def verify(identities, catalog=None, assign=None):
    """Split the identities missing aff.id into the fillable and the rest.

    The gate is "nothing fillable is left", not "every identity carries aff".
    A live population holds identities this script cannot resolve — no
    org.short_tag, a tag the catalog does not know (a bare person slug), an
    absorbed org — and no run changes that. Counted as failures they would keep
    --verify at exit 1 forever, and a gate that can never go green is not a
    gate. They are reported with the reason instead: each one is a --assign or
    a catalog fix, a decision for the operator, not a backfill.

    Returns (fillable, unresolvable): a list of labels, and a list of
    (label, why). Without a catalog the split cannot be made, so every missing
    identity comes back fillable and the caller says the count is unsplit.
    """
    missing = [identity for identity in identities if not current_aff_id(identity)]
    if catalog is None:
        return [_label(identity) for identity in missing], []
    # Every row here lacks aff.id, so plan() skips one only because it cannot
    # resolve an org for it — the planner is the authority on what is fillable,
    # rather than a second copy of the same rules that can drift from it.
    actions, skipped = plan(missing, catalog, assign=assign)
    return [action["email"] for action in actions], [(row["email"], row["why"]) for row in skipped]


def report_verify(identities, catalog, assign):
    fillable, unresolvable = verify(identities, catalog, assign)
    if catalog is None:
        print(f"{len(identities)} identities; {len(fillable)} without aff.id "
              "(no --catalog or --env: the fillable are not told from the unresolvable)")
    else:
        print(f"{len(identities)} identities; {len(fillable)} fillable but missing aff.id, "
              f"{len(unresolvable)} with no resolvable org")
    for email in fillable:
        print(f"  missing  {email}")
    for email, why in unresolvable:
        print(f"  no org   {email:<44} {why}")
    return 1 if fillable else 0


def run_backfill(admin, identities, catalog, assign, only, apply):
    actions, skipped = plan(identities, catalog, only=only, assign=assign)
    for row in skipped:
        print(f"skip   {row['email']:<44} {row['why']}")
    for action in actions:
        state = "set" if apply else "would set"
        aff = action["aff"]
        print(f"{state:<9} {action['email']:<44} aff {aff['id']} {aff['kind']} {aff.get('nation', '-')}"
              f"  (from {action['source']} {action['tag']!r})")
    if not apply:
        print(f"{len(actions)} to set, {len(skipped)} skipped; dry run, nothing written")
        return 0
    failures = sum(1 for action in actions if not apply_one(admin, action))
    print(f"{len(actions) - failures} set, {failures} failed, {len(skipped)} skipped")
    return 2 if failures else 0


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--admin", default=DEFAULT_ADMIN, help="Kratos admin base URL")
    parser.add_argument("--catalog", help="org catalog file, JSON or the generated YAML")
    parser.add_argument("--env", help="read orgs/<env>.json, else orgs/<env>.yaml, next to this script")
    parser.add_argument("--apply", action="store_true", help="write; the default is a dry run")
    parser.add_argument("--only", help="limit to one identity by email (a trial run)")
    parser.add_argument("--assign", action="append", metavar="EMAIL=ORG_ID",
                        help="name the org for an identity the catalog cannot resolve; repeatable")
    parser.add_argument("--verify", action="store_true",
                        help="report counts; exit 1 while an identity the catalog can fill lacks aff.id")
    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    assign = parse_assignments(args.assign)
    path = catalog_path(args.catalog, args.env)
    if not path and not args.verify:
        parser.error("one of --catalog or --env is required")
    # An --assign is checked against the catalog before anything is read or
    # written, the same refusal under --verify as under --apply: a preview that
    # counted a typo as "no resolvable org" could exit 0 on input --apply refuses.
    if assign and not path:
        parser.error("--assign needs --catalog or --env to check the org it names")
    catalog = load_catalog(path) if path else None
    if path:
        print(f"catalog {path}")
    unknown = unknown_assignments(assign, catalog or {})
    if unknown:
        parser.error(f"--assign {', '.join(unknown)}: not in the catalog")

    try:
        identities = list_identities(args.admin)
    except (urllib.error.URLError, OSError, ValueError) as exc:
        print(f"cannot list identities from {args.admin}: {exc}", file=sys.stderr)
        return 2

    if args.verify:
        return report_verify(identities, catalog, assign)
    return run_backfill(args.admin, identities, catalog, assign, args.only, args.apply)


if __name__ == "__main__":
    sys.exit(main())
