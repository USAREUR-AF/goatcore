#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""netbird-service-identities — check a goatnet's NetBird service identities,
and mint the ones it is missing.

Registry: ops/netbird/service-identities.yaml. Design:
docs/design/netbird-service-identities.md.

A goatnet's group_vars decide which identities it needs: an identity is
required when its `required_when` flag is true for that goatnet (or always).
For each one this reports whether the token is in the operator environment
and, with --live, whether it authenticates as the service user the registry
names.

    ops/scripts/netbird-service-identities.py zagros-dev            # check
    ops/scripts/netbird-service-identities.py zagros-dev --live     # + verify against NetBird
    ops/scripts/netbird-service-identities.py zagros-dev --mint     # create what is missing

--mint is the operator deciding to create admin credentials on a goatnet, which
is why it is a command and never a playbook side effect. It needs the bootstrap
identity's token (netbird-infra, minted by hand in standup phase 11). For each
required identity whose token is absent it reuses the service user if it
exists, otherwise creates it (role admin), mints a 365-day token and stores it
under the declared secrets posture (ops/secrets/posture.py, ADR 1204):

  1password   the export is appended to .envrc.local.d/30-site-services.envrc in
              the primary checkout AND the value goes into the registry's
              1Password item (--no-1password skips the item)
  env-file, sops, git-crypt
              the value goes into the posture's own file through posture.set_values;
              no envrc is written and 1Password is never contacted
  openbao, custom, env
              the export is appended to the envrc as above; the store holds no
              copy, and the report says so

It never prints a token and never replaces a token that is already set.

Exit: 0 every required identity present (and verified, with --live)
      1 at least one required identity missing, unverified or failed to mint
      2 usage, registry or group_vars error
"""
import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("netbird-service-identities: needs PyYAML (pip3 install pyyaml)")

REPO = Path(__file__).resolve().parents[2]
REGISTRY = REPO / "ops" / "netbird" / "service-identities.yaml"
GV = REPO / "ops" / "ansible" / "group_vars"
ROLES = REPO / "ops" / "ansible" / "roles"
TOKEN_DAYS = 365
EXPIRY_WARN_DAYS = 30


def die(msg):
    print(f"netbird-service-identities: {msg}", file=sys.stderr)
    sys.exit(2)


_POSTURE = None


def posture():
    """ops/secrets/posture.py as a module — the one place every caller asks which
    secrets store is in force (ADR 1204)."""
    global _POSTURE
    if _POSTURE is None:
        import importlib.util
        path = REPO / "ops" / "secrets" / "posture.py"
        spec = importlib.util.spec_from_file_location("goat_posture", path)
        mod = importlib.util.module_from_spec(spec)
        try:
            spec.loader.exec_module(mod)
        except (OSError, ImportError) as e:
            die(f"cannot load the secrets posture ({path}): {e}")
        _POSTURE = mod
    return _POSTURE


def secrets_store():
    p = posture()
    try:
        return p.store(str(REPO))
    except p.PostureError as e:
        die(str(e))


# ---------------------------------------------------------------------------
# Registry and goatnet config
# ---------------------------------------------------------------------------

def site_upper(goatnet):
    return goatnet.upper().replace("-", "_")


def load_registry():
    spec = yaml.safe_load(REGISTRY.read_text())
    if spec.get("schema") != "netbird-service-identities/v1":
        die(f"{REGISTRY}: unknown schema {spec.get('schema')!r}")
    return spec


def load_group(group):
    d = GV / group
    if not d.is_dir():
        die(f"no group_vars directory {d} (pass --group)")
    merged = {}
    for f in sorted(d.glob("*.yml")):
        data = yaml.safe_load(f.read_text()) or {}
        if isinstance(data, dict):
            merged.update(data)
    return merged


def role_default(role, var):
    f = ROLES / role / "defaults" / "main.yml"
    if not f.is_file():
        return None
    return (yaml.safe_load(f.read_text()) or {}).get(var)


def as_bool(value, where):
    if isinstance(value, bool):
        return value
    if isinstance(value, str) and value.strip().lower() in ("true", "yes", "1"):
        return True
    if isinstance(value, str) and value.strip().lower() in ("false", "no", "0", ""):
        return False
    die(f"{where} is {value!r}, not a literal boolean; this script evaluates "
        f"required_when offline and cannot render Jinja")


def requirement(ident, gvars):
    """(required, reason) for one identity on this goatnet."""
    rw = ident.get("required_when")
    if rw == "always":
        return True, "always"
    var, role = rw["var"], rw["role"]
    if var in gvars:
        return as_bool(gvars[var], f"group_vars {var}"), f"{var} (group_vars)"
    default = role_default(role, var)
    if default is None:
        return False, f"{var} unset"
    return as_bool(default, f"roles/{role} default {var}"), f"{var} (role default)"


def expand(template, goatnet):
    return template.replace("{goatnet}", goatnet).replace("{SITE}", site_upper(goatnet))


def service_user_name(key, ident, goatnet, gvars):
    """The goatnet's own name for the identity's service user: its group_vars
    netbird_service_user_names[<identity>], then the registry's declared_names
    for the goatnet, then the registry's service_user pattern."""
    own = (gvars.get("netbird_service_user_names") or {}).get(key)
    return own or (ident.get("declared_names") or {}).get(goatnet) or expand(ident["service_user"], goatnet)


# ---------------------------------------------------------------------------
# Operator environment
# ---------------------------------------------------------------------------

def primary_checkout():
    common = subprocess.run(
        ["git", "-C", str(REPO), "rev-parse", "--path-format=absolute", "--git-common-dir"],
        capture_output=True, text=True, check=True).stdout.strip()
    return Path(common).parent


def envrc_values(names, envrc_dir):
    """Values for `names`: the process environment first, then the primary
    checkout's .envrc.local.d files sourced in a subshell (the same files
    ansible-preflight.sh sources). Values are returned, never printed."""
    found = {n: os.environ[n] for n in names if os.environ.get(n)}
    # Under a value store the posture's own file holds the tokens (that is where
    # --mint writes them), so it is read before the legacy envrc files.
    p = posture()
    if secrets_store() in p.VALUE_STORES:
        try:
            for n, v in p.values(str(REPO)).items():
                if n in names and v and n not in found:
                    found[n] = v
        except p.PostureError:
            pass  # no store file yet: nothing minted under this posture so far
    missing = [n for n in names if n not in found]
    files = sorted(envrc_dir.glob("*.envrc")) if envrc_dir.is_dir() else []
    if missing and files:
        script = "set -a; " + "; ".join(f". {json.dumps(str(f))} >/dev/null 2>&1" for f in files) + \
                 "; set +a; python3 -c 'import json,os,sys; print(json.dumps({n: os.environ.get(n, \"\") for n in sys.argv[1:]}))' " + \
                 " ".join(missing)
        out = subprocess.run(["bash", "-c", script], capture_output=True, text=True).stdout
        try:
            for n, v in json.loads(out.strip().splitlines()[-1]).items():
                if v:
                    found[n] = v
        except (ValueError, IndexError):
            pass  # cannot parse sourced values: retain values already found in the environment
    return found


def append_export(envrc_file, name, value, identity, user):
    text = envrc_file.read_text() if envrc_file.exists() else ""
    for line in text.splitlines():
        if line.strip().startswith((f"export {name}=", f"{name}=")):
            raise RuntimeError(f"{envrc_file} already declares {name}; refusing to overwrite it")
    stamp = dt.date.today().isoformat()
    block = (f"\n# NetBird service identity '{identity}' (service user {user}), minted {stamp} by "
             f"ops/scripts/netbird-service-identities.py\nexport {name}={json.dumps(value)}\n")
    envrc_file.parent.mkdir(parents=True, exist_ok=True)
    with open(envrc_file, "a") as f:
        f.write(block)
    os.chmod(envrc_file, 0o600)


# ---------------------------------------------------------------------------
# NetBird API
# ---------------------------------------------------------------------------

class NetBird:
    def __init__(self, base, token, site=""):
        self.base = base.rstrip("/")
        self.token = token
        # Verify against the goatnet's own CA when its root certificate is in
        # the repository; a goatnet behind a public CA verifies with the
        # system store. Verification is never turned off.
        import ssl
        ca = REPO / "ops/enrollment/ca" / site / "root-cert.pem" if site else None
        # NETBIRD_MGMT_CA=public: a goatnet with its own CA whose mgmt API carries a
        # public ACME certificate (internet profile) verifies with the system store.
        if os.environ.get("NETBIRD_MGMT_CA", "") == "public":
            ca = None
        self.tls = ssl.create_default_context(cafile=str(ca)) if ca and ca.is_file() else ssl.create_default_context()

    def call(self, method, path, body=None, token=None):
        req = urllib.request.Request(
            self.base + path, method=method,
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Authorization": f"Token {token or self.token}",
                     "Content-Type": "application/json", "Accept": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=20, context=self.tls) as r:
                raw = r.read()
                return r.status, (json.loads(raw) if raw else None)
        except urllib.error.HTTPError as e:
            return e.code, None
        except (urllib.error.URLError, TimeoutError) as e:
            raise RuntimeError(f"{self.base}{path}: {e}") from e

    def users(self):
        status, body = self.call("GET", "/api/users?service_user=true")
        if status != 200:
            raise RuntimeError(f"listing service users returned HTTP {status} — is the bootstrap token admin?")
        return {u["name"]: u for u in body}

    def tokens(self, user_id):
        status, body = self.call("GET", f"/api/users/{user_id}/tokens")
        return body if status == 200 and isinstance(body, list) else []


def parse_ts(value):
    if not value:
        return None
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def verify_owner(nb, token, expected_user, users):
    """Prove which service user a token belongs to without seeing token ids.

    NetBird never returns a PAT's value or maps a value to its id, but it does
    stamp `last_used` on the token record when a request authenticates. So:
    authenticate one request with the token, then look for the expected user's
    token whose last_used moved past that moment. Returns (ok, detail)."""
    before = dt.datetime.now(dt.timezone.utc) - dt.timedelta(seconds=1)
    status, _ = nb.call("GET", "/api/groups", token=token)
    if status != 200:
        return False, f"token does not authenticate (HTTP {status})"
    user = users.get(expected_user)
    if user is None:
        return False, f"authenticates, but service user {expected_user} does not exist"
    for _ in range(3):
        time.sleep(1)
        for t in nb.tokens(user["id"]):
            used = parse_ts(t.get("last_used"))
            if used and used >= before:
                expires = parse_ts(t.get("expiration_date"))
                days = (expires - dt.datetime.now(dt.timezone.utc)).days if expires else None
                note = f"belongs to {expected_user}"
                if days is not None:
                    note += f", expires in {days} d"
                    if days < EXPIRY_WARN_DAYS:
                        return False, note + f" (under {EXPIRY_WARN_DAYS} d: rotate it)"
                return True, note
    return False, f"authenticates, but not as {expected_user} (another identity's token?)"


# ---------------------------------------------------------------------------
# 1Password
# ---------------------------------------------------------------------------

def onepassword_account(spec):
    """The 1Password account the item lives in: the one the secrets posture pins
    (OP_ACCOUNT, else `secrets: op_account:` in goatbuilder.yaml, ADR 1204),
    else the registry's onepassword.account."""
    return posture().op_account(str(REPO)) or (spec.get("onepassword") or {}).get("account", "")


def store_in_1password(spec, ident, goatnet, value):
    """Make the registry's 1Password item hold `value`. Writes go through a JSON
    template on stdin so the secret never appears on disk or a command line.
    Never overwrites a different value already stored. Returns
    (ok, status)."""
    op = spec.get("onepassword") or {}
    where = ident.get("onepassword")
    if not where or not op:
        return True, "no 1Password item declared"
    item, field = expand(where["item"], goatnet), where["field"]
    base = ["op", "--account", onepassword_account(spec)]
    got = subprocess.run(base + ["item", "get", item, "--vault", op["vault"], "--format", "json"],
                         capture_output=True, text=True)
    if got.returncode == 0:
        doc = json.loads(got.stdout)
        current = next((f.get("value") for f in doc.get("fields", []) if f.get("label") == field), None)
        if current == value:
            return True, f"already in 1Password {op['vault']}/{item} field {field}"
        if current:
            return False, (f"1Password {op['vault']}/{item} field {field} holds a DIFFERENT value; "
                           f"left untouched — decide which token is live before changing it")
        doc["fields"] = [f for f in doc.get("fields", []) if f.get("label") != field] + \
            [{"label": field, "type": "CONCEALED", "value": value}]
        action = ["item", "edit", doc["id"], "--vault", op["vault"]]
    else:
        # 1Password refuses to create a PASSWORD item with an empty password
        # field, so a new item carries the token there as well as in the named
        # field the registry declares (the field consumers read).
        doc = {"title": item, "category": "PASSWORD",
               "fields": [{"id": "password", "type": "CONCEALED", "purpose": "PASSWORD", "label": "password", "value": value},
                          {"label": field, "type": "CONCEALED", "value": value}]}
        action = ["item", "create", "--vault", op["vault"], "-"]
    # `op item create -` and `op item edit <id>` both accept JSON on stdin.
    # Keep the document off disk, argv, stdout, and diagnostic messages.
    res = subprocess.run(base + action, input=json.dumps(doc), capture_output=True, text=True)
    if res.returncode != 0:
        # The CLI can echo submitted fields in an error. Report the status,
        # not stderr from a process that received the credential document.
        return False, f"NOT stored in 1Password {op['vault']}/{item} (op exited {res.returncode})"
    return True, f"stored in 1Password {op['vault']}/{item} field {field}"


# ---------------------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("goatnet", help="the goatnet's site_id, e.g. zagros-dev")
    ap.add_argument("--group", help="group_vars directory (default <goatnet>_hosts)")
    ap.add_argument("--live", action="store_true", help="verify each token against NetBird")
    ap.add_argument("--mint", action="store_true", help="create the missing identities (implies --live)")
    ap.add_argument("--only", metavar="IDENTITY", help="limit --mint and the report to one identity key")
    ap.add_argument("--no-1password", action="store_true",
                    help="with --mint, skip the 1Password write (implied whenever the declared secrets "
                         "posture is not 1password)")
    ap.add_argument("--sync-1password", action="store_true",
                    help="store every required token already in the environment into its 1Password item "
                         "(fills gaps, never overwrites a different value)")
    a = ap.parse_args()

    spec = load_registry()
    # The declared secrets posture decides where a minted token is stored. Off the
    # 1Password posture nothing is written to 1Password (goatlite.md §5 row 4) and
    # the token goes where the posture keeps values.
    store = secrets_store()
    p = posture()
    if store != "1password":
        if a.sync_1password:
            die(f"--sync-1password: the declared secrets posture is {store}, not 1password — there is no "
                "1Password item to sync (GOAT_SECRET_STORE / secrets.store in goatbuilder.yaml)")
        a.no_1password = True
    group = a.group or a.goatnet.replace("-", "_") + "_hosts"
    gvars = load_group(group)
    if gvars.get("site_id") != a.goatnet:
        die(f"group_vars/{group} has site_id {gvars.get('site_id')!r}, not {a.goatnet!r}")
    fqdn = gvars.get("public_mgmt_fqdn")
    if not fqdn:
        die(f"group_vars/{group} has no public_mgmt_fqdn")
    mgmt = f"https://{fqdn}"
    envrc_dir = primary_checkout() / ".envrc.local.d"

    idents = spec["identities"]
    bootstrap = [k for k, v in idents.items() if v.get("bootstrap")]
    if len(bootstrap) != 1:
        die("the registry must declare exactly one bootstrap identity")
    if a.only and a.only not in idents:
        die(f"--only {a.only}: not an identity in {REGISTRY} ({', '.join(idents)})")
    rows = []
    for key, ident in idents.items():
        if a.only and key != a.only and not ident.get("bootstrap"):
            continue
        required, why = requirement(ident, gvars)
        rows.append({"key": key, "ident": ident, "required": required, "why": why,
                     "env": expand(ident["token_env"], a.goatnet),
                     "user": service_user_name(key, ident, a.goatnet, gvars)})
    values = envrc_values([r["env"] for r in rows], envrc_dir)

    boot = next(r for r in rows if r["key"] == bootstrap[0])
    nb = None
    if a.live or a.mint:
        if boot["env"] not in values:
            die(f"{boot['env']} (the bootstrap identity) is not set. Mint it by hand as "
                f"docs/operations/goatnet-standup.md Phase 11 describes, then re-run.")
        nb = NetBird(mgmt, values[boot["env"]], site=a.goatnet)

    failed = False
    op_failed = False
    notes = []
    stored = set()
    if a.mint:
        envrc_file = envrc_dir / "30-site-services.envrc"
        users = nb.users()
        for r in rows:
            if not r["required"] or r["env"] in values or r["ident"].get("bootstrap"):
                continue
            try:
                user = users.get(r["user"])
                if user is None:
                    status, user = nb.call("POST", "/api/users", {
                        "name": r["user"], "role": "admin", "is_service_user": True, "auto_groups": []})
                    if status not in (200, 201) or not user:
                        raise RuntimeError(f"creating service user {r['user']} returned HTTP {status}")
                    users[r["user"]] = user
                status, tok = nb.call("POST", f"/api/users/{user['id']}/tokens",
                                      {"name": r["ident"]["token_name"], "expires_in": TOKEN_DAYS})
                if status not in (200, 201) or not tok or not tok.get("plain_token"):
                    raise RuntimeError(f"minting a token for {r['user']} returned HTTP {status}")
                value = tok["plain_token"]
                if store in p.VALUE_STORES:
                    try:
                        p.set_values({r["env"]: value}, str(REPO))
                    except p.PostureError as e:
                        raise RuntimeError(f"token minted but not stored under the {store} posture: {e}") from e
                    where = f"the {store} store ({os.path.relpath(p.store_file(str(REPO)), REPO)})"
                else:
                    append_export(envrc_file, r["env"], value, r["key"], r["user"])
                    where = str(envrc_file)
                values[r["env"]] = value
                if store != "1password":
                    op_ok, op_note = True, (f"1Password not used: the declared secrets posture is {store}"
                                            + ("" if store in p.VALUE_STORES else
                                               f"; the {store} store holds no copy of this token — add one there by hand if it should"))
                elif a.no_1password:
                    op_ok, op_note = True, "1Password skipped (--no-1password)"
                else:
                    op_ok, op_note = store_in_1password(spec, r["ident"], a.goatnet, value)
                    stored.add(r["key"])
                notes.append(f"  minted {r['key']}: service user {r['user']}, {r['env']} written to "
                             f"{where}; {op_note}")
                if not op_ok:
                    op_failed = True
            except RuntimeError as e:
                notes.append(f"  FAILED to mint {r['key']}: {e}")
                failed = True

    if a.sync_1password:
        for r in rows:
            if r["required"] and r["env"] in values and r["key"] not in stored:
                op_ok, op_note = store_in_1password(spec, r["ident"], a.goatnet, values[r["env"]])
                notes.append(f"  {r['key']}: {op_note}")
                op_failed |= not op_ok

    users = nb.users() if nb else {}
    table = []
    for r in rows:
        req = ("yes: " if r["required"] else "no: ") + r["why"]
        if r["env"] not in values:
            state = "MISSING" if r["required"] else "not set"
            failed |= r["required"]
        elif nb:
            ok, detail = verify_owner(nb, values[r["env"]], r["user"], users)
            state = ("verified, " if ok else "UNVERIFIED, ") + detail
            failed |= r["required"] and not ok
        else:
            state = "set (not verified; add --live)"
        table.append((r["key"], req, r["user"], r["env"], state))
    head = ("identity", "required", "service user", "token variable", "state")
    widths = [max(len(row[i]) for row in table + [head]) for i in range(4)]
    print(f"NetBird service identities: {a.goatnet} ({mgmt}), group_vars/{group}")
    for row in [head] + table:
        print("  ".join(c.ljust(w) for c, w in zip(row[:4], widths)) + "  " + row[4])
    for line in notes:
        print(line)
    if failed:
        print(f"\nFAIL: a required identity is missing or unverified. Create what is missing with\n"
              f"  ops/scripts/netbird-service-identities.py {a.goatnet} --mint")
        return 1
    if op_failed:
        print(f"\nFAIL: the identities work but 1Password does not hold every token. Fix the cause above, then\n"
              f"  ops/scripts/netbird-service-identities.py {a.goatnet} --sync-1password")
        return 1
    print("\nOK: every required identity is present" + (" and verified." if nb else "."))
    return 0


if __name__ == "__main__":
    sys.exit(main())
