#!/usr/bin/env python3
"""posture.py — the declared secrets posture, and the one place every caller asks about it.

A team chooses how its deploy secrets are held (ADR 1204). goat's own choice is
references only, resolved by 1Password per command (ADR 1186); a team adopting
the repository may choose a simpler store and owns that trade. Both avenues —
the scripts (rc.sh, deploy-component.sh, ansible-preflight.sh, with-secrets) and
goatbuilder — read the posture through this file, so they cannot disagree about
which store is in force, which variables are available, or what to tell the
operator when one is missing.

Where the posture comes from, first match wins:
  1. GOAT_SECRET_STORE in the environment (a per-command override)
  2. a goatlite checkout (ADR 1213): `secrets: store:` in <repo>/goatlite.yaml when
     that file declares `preset: lite`, else the preset's own choice, env-file —
     the learner's one file outranks the repository's generated goatbuilder.yaml
  3. `secrets: store:` in ops/goatbuilder/goatbuilder.yaml (the declared posture)
  4. 1password

Stores:
  1password  references (op://) in the environment, resolved by `op run`   (goat's posture)
  openbao    references (bao://), resolved by ops/secrets/openbao/run      (ADR 1203)
  vaultwarden references (bw://), resolved by ops/secrets/vaultwarden/run  (ADR 1212)
  custom     references, resolved by $GOAT_SECRET_RUN
  sops       values in a sops-encrypted dotenv file, decrypted per command
  git-crypt  values in a dotenv file encrypted in git by git-crypt, readable once unlocked
  env-file   values in a plaintext dotenv file: gitignored, mode 0600, never committed
  env        values already exported in the shell; nothing is resolved

usage:
  posture.py store                       print the store in force
  posture.py available                   print every variable name that has a value or a reference
  posture.py missing NAME...             print the names with neither; exit 1 if any
  posture.py hint NAME...                print how to supply those names under this posture
  posture.py op-account                  print the 1Password account to pin (OP_ACCOUNT, else
                                         declared); exit 3 if unset and >1 account is configured
  posture.py doctor                      print `ok|FAIL <check> <detail>` rows for the posture
  posture.py exec -- CMD...              run CMD with the posture's values in its environment
  posture.py init --env GOATNET [--file PATH] [--force] [--if-env-file]
                                         write an env file with strong random values for every
                                         variable the goatnet's plays template (zero-SaaS quickstart);
                                         --if-env-file changes nothing under any other store
  posture.py set --stdin-json            store the values of a JSON object {NAME: value} read from
                                         stdin under a VALUE store (env-file, git-crypt, sops); a
                                         name that already holds a value is kept, never rotated;
                                         prints NAME<TAB>kept|written<TAB>where, never a value
"""
from __future__ import annotations

import json
import os
import re
import secrets as _secrets
import uuid as _uuid
import shutil
import stat
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

STORES = ("1password", "openbao", "vaultwarden", "custom", "sops", "git-crypt", "env-file", "env")
REFERENCE_STORES = ("1password", "openbao", "vaultwarden", "custom")
VALUE_STORES = ("sops", "git-crypt", "env-file")
DEFAULT_PREFIX = {"1password": "op://", "openbao": "bao://", "vaultwarden": "bw://"}
DEFAULTS = {
    "env_file": ".envrc.local.d/secrets.env",
    "sops_file": "secrets.sops.env",
    "git_crypt_file": "secrets/goat.env",
}
ENV_OVERRIDE = {
    "store": "GOAT_SECRET_STORE",
    "env_file": "GOAT_SECRET_ENV_FILE",
    "sops_file": "GOAT_SECRET_SOPS_FILE",
    "git_crypt_file": "GOAT_SECRET_GITCRYPT_FILE",
    "ref_prefix": "GOAT_SECRET_REF_PREFIX",
    # The 1Password account op:// references resolve against. With an unset
    # OP_ACCOUNT and more than one account configured, `op` prompts across ALL
    # of them (ADR 1186); pinning it per command keeps the resolve on one.
    "op_account": "OP_ACCOUNT",
}
NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
GITCRYPT_MAGIC = b"\x00GITCRYPT\x00"


class PostureError(Exception):
    pass


def _read(path: str) -> str:
    with open(path, encoding="utf-8") as fh:
        return fh.read()


# --------------------------------------------------------------------------- declaration

def declared(repo: str = REPO) -> dict:
    """The `secrets:` block of goatbuilder.yaml, read without a YAML dependency.

    The file is generated (ops/goatbuilder/gen-config.py) and its `secrets:`
    block is flat `key: value` pairs, so a line reader is exact here and keeps
    with-secrets usable on a laptop that has no PyYAML.
    """
    path = os.environ.get("GOAT_BUILDER_CONFIG") or os.path.join(repo, "ops/goatbuilder/goatbuilder.yaml")
    out = _flat_block(path, "secrets:")
    lite = _lite_declared(repo)
    if lite:
        # goatlite.yaml is the more specific declaration: its keys win, the
        # generated goatbuilder.yaml fills in anything it does not say.
        out = {**{k: v for k, v in out.items() if v}, **lite}
    return out


def _flat_block(path: str, header: str) -> dict:
    """The flat `key: value` pairs under a top-level `header` line of a YAML file, read
    without a YAML dependency; {} when the file or the block is absent."""
    out: dict = {}
    try:
        lines = _read(path).splitlines()
    except OSError:
        return out
    inside = False
    for line in lines:
        if not inside:
            inside = line.rstrip() == header
            continue
        if line and not line.startswith(" "):
            break
        m = re.match(r"^\s+([a-z_]+):\s*(.*?)\s*$", line)
        if m:
            out[m.group(1)] = m.group(2).strip("'\"")
    return out


def lite_config_path(repo: str = REPO) -> str:
    """<repo>/goatlite.yaml, the learner's four-fact file, which outranks goatbuilder.yaml.
    GOAT_LITE_CONFIG names another path, or a path that does not exist to read this
    checkout as not a goatlite: goat's own tests ask for goat's declared store while
    a learner's goatlite.yaml may sit at the root of the same checkout."""
    override = os.environ.get("GOAT_LITE_CONFIG")
    if override is not None:
        return override
    return os.path.join(repo, "goatlite.yaml")


def _lite_declared(repo: str) -> dict:
    """The secrets posture a goatlite checkout declares (ADR 1213 D4): the `secrets:` block
    of <repo>/goatlite.yaml when that file says `preset: lite`, else the preset's own
    default, env-file. {} when the checkout is not a goatlite."""
    path = lite_config_path(repo)
    try:
        text = _read(path)
    except OSError:
        return {}
    if not re.search(r"^preset:\s*['\"]?lite['\"]?\s*$", text, re.M):
        return {}
    out = _flat_block(path, "secrets:")
    out.setdefault("store", "env-file")
    return out


def lite_name(repo: str = REPO) -> str:
    """The goatnet a goatlite checkout declares (`lite.name` in goatlite.yaml), or ''."""
    if not _lite_declared(repo):
        return ""
    name = _flat_block(lite_config_path(repo), "lite:").get("name", "")
    return re.split(r"\s+#", name, maxsplit=1)[0].strip().strip("'\"")


def _first_standup(goatnet: str) -> str:
    return (f"phase 1 of the standup renders the goatlite {goatnet} and creates the local secrets "
            f"store with strong random values: goatbuilder standup {goatnet} --to 1")


def declared_source(repo: str = REPO) -> str:
    """Where the store in force was declared: environment | goatbuilder.yaml | goatlite.yaml | default."""
    if os.environ.get(ENV_OVERRIDE["store"]):
        return "environment"
    if _lite_declared(repo).get("store"):
        return "goatlite.yaml"
    path = os.environ.get("GOAT_BUILDER_CONFIG") or os.path.join(repo, "ops/goatbuilder/goatbuilder.yaml")
    if _flat_block(path, "secrets:").get("store"):
        return "goatbuilder.yaml"
    return "default"


def setting(key: str, repo: str = REPO) -> str:
    env = os.environ.get(ENV_OVERRIDE.get(key, ""), "")
    if env:
        return env
    value = declared(repo).get(key, "")
    if value:
        return value
    if key == "store":
        # Nothing declared: the store that needs no account of anyone's, so a
        # checkout nobody has configured still resolves its own values
        # (`goatbuilder secrets init` writes them). A project that uses a
        # service declares it in ops/goatbuilder/posture.yml (ADR 1204).
        return "env-file"
    if key == "ref_prefix":
        return DEFAULT_PREFIX.get(setting("store", repo), "op://")
    return DEFAULTS.get(key, "")


def store(repo: str = REPO) -> str:
    s = setting("store", repo)
    if s not in STORES:
        raise PostureError(f"unknown secrets store '{s}' — one of: {', '.join(STORES)}")
    return s


def store_file(repo: str = REPO) -> str:
    """The values file for a value store, absolute; '' for the others."""
    key = {"env-file": "env_file", "sops": "sops_file", "git-crypt": "git_crypt_file"}.get(store(repo))
    if not key:
        return ""
    p = setting(key, repo)
    if os.path.isabs(p):
        return p
    here = os.path.join(repo, p)
    if os.path.exists(here):
        return here
    # A linked worktree has no gitignored files of its own: the operator's file
    # lives in the main checkout, as .envrc.local.d does (ansible-preflight.sh).
    main = _main_checkout(repo)
    if main and os.path.exists(os.path.join(main, p)):
        return os.path.join(main, p)
    return here


def _main_checkout(repo: str) -> str:
    try:
        common = subprocess.run(["git", "-C", repo, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                                capture_output=True, text=True).stdout.strip()
    except OSError:
        return ""
    return os.path.dirname(common) if common else ""


# --------------------------------------------------------------------------- 1Password account

def op_account(repo: str = REPO) -> str:
    """The 1Password account op:// references resolve against, or ''.
    OP_ACCOUNT in the environment wins (a per-command override), then
    `secrets: op_account:` in goatbuilder.yaml (the declared posture)."""
    return setting("op_account", repo)


def _op_account_count() -> int:
    """How many 1Password accounts `op` has configured on this machine, or -1 if
    it cannot tell (op absent, errored, timed out). `op account list` reads local
    config only — it needs no session and never prompts."""
    if not shutil.which("op"):
        return -1
    try:
        r = subprocess.run(["op", "account", "list", "--format=json"],
                           capture_output=True, text=True, timeout=15,
                           stdin=subprocess.DEVNULL)
    except (OSError, subprocess.SubprocessError):
        return -1
    if r.returncode != 0:
        return -1
    try:
        data = json.loads(r.stdout or "[]")
    except ValueError:
        return -1
    return len(data) if isinstance(data, list) else -1


def _op_account_shorthands() -> list:
    if not shutil.which("op"):
        return []
    try:
        r = subprocess.run(["op", "account", "list", "--format=json"],
                           capture_output=True, text=True, timeout=15,
                           stdin=subprocess.DEVNULL)
        data = json.loads(r.stdout or "[]")
    except (OSError, subprocess.SubprocessError, ValueError):
        return []
    out = []
    for a in data if isinstance(data, list) else []:
        out.append(a.get("shorthand") or a.get("url") or a.get("account_uuid") or "?")
    return out


def op_account_resolve(repo: str = REPO) -> tuple:
    """Resolve the account to pin before any `op` call, fail-closed on ambiguity.

    Returns (account, error):
      - (account, None)  pin OP_ACCOUNT=account for this command
      - ("", None)       nothing to pin and nothing ambiguous (0 or 1 account, or
                         op cannot be queried — a missing op is caught elsewhere)
      - ("", message)    REFUSE: OP_ACCOUNT is unset and more than one account is
                         configured, so `op` would prompt across every one
    """
    acct = op_account(repo)
    if acct:
        return acct, None
    count = _op_account_count()
    if count > 1:
        names = _op_account_shorthands()
        shown = f" ({', '.join(names)})" if names else ""
        msg = (f"op:// references, but OP_ACCOUNT is unset and this machine has {count} "
               f"1Password accounts configured{shown} — `op` would prompt across every one. "
               "Set OP_ACCOUNT=<your account> for this command (the goat team store is "
               "example.1password.com), or declare it once as `secrets: {op_account: <account>}` "
               "in ops/goatbuilder/goatbuilder.yaml.")
        return "", msg
    return "", None


# --------------------------------------------------------------------------- reading values

def parse_dotenv(text: str) -> dict:
    """KEY=value lines; `export ` allowed; one level of matching quotes stripped; # comments."""
    out = {}
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export "):].lstrip()
        key, sep, value = line.partition("=")
        key = key.strip()
        if not sep or not NAME.match(key):
            continue
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
            value = value[1:-1]
        out[key] = value
    return out


def _sops_names(path: str) -> set:
    """sops encrypts values, not keys: the names are readable without the key."""
    try:
        text = _read(path)
    except OSError:
        return set()
    return {k for k in parse_dotenv(text) if not k.startswith("sops_")}


def _is_git_crypt_locked(path: str) -> bool:
    try:
        with open(path, "rb") as fh:
            return fh.read(len(GITCRYPT_MAGIC)) == GITCRYPT_MAGIC
    except OSError:
        return False


def values(repo: str = REPO) -> dict:
    """The posture's values, for exec. Reference stores and env return {}: their values
    are the environment (references are resolved by the store's own injector)."""
    s = store(repo)
    path = store_file(repo)
    if s == "env-file":
        if not os.path.isfile(path):
            lite = lite_name(repo)
            if lite:
                raise PostureError(f"env-file store: {path} does not exist — {_first_standup(lite)}")
            raise PostureError(f"env-file store: {path} does not exist — create it with: goatbuilder secrets init --env <goatnet>")
        return parse_dotenv(_read(path))
    if s == "git-crypt":
        if not os.path.isfile(path):
            raise PostureError(f"git-crypt store: {path} does not exist")
        if _is_git_crypt_locked(path):
            raise PostureError(f"git-crypt store: {path} is still encrypted — run: git-crypt unlock")
        return parse_dotenv(_read(path))
    if s == "sops":
        if not shutil.which("sops"):
            raise PostureError("sops store: the sops CLI is not installed")
        r = subprocess.run(["sops", "-d", "--input-type", "dotenv", "--output-type", "dotenv", path],
                           capture_output=True, text=True)
        if r.returncode != 0:
            raise PostureError(f"sops store: could not decrypt {path}: {r.stderr.strip()[:300]}")
        return parse_dotenv(r.stdout)
    return {}


def available(repo: str = REPO) -> set:
    """Every variable name with a value or a reference under this posture, without
    decrypting anything: presence is what the secrets preflights need."""
    s = store(repo)
    # Under a value store (or env) nothing resolves a reference, so a variable whose
    # value is still an op:// or bao:// pointer is not available, whatever the shell says.
    refs = ("op://", "bao://", "bw://") if s not in REFERENCE_STORES else ()
    names = {k for k, v in os.environ.items() if v and not (refs and v.startswith(refs))}
    path = store_file(repo)
    if s in ("env-file", "git-crypt") and os.path.isfile(path) and not _is_git_crypt_locked(path):
        names |= {k for k, v in parse_dotenv(_read(path)).items() if v}
    elif s == "sops" and os.path.isfile(path):
        names |= _sops_names(path)
    return names


# --------------------------------------------------------------------------- telling the operator

def hint(names: list, repo: str = REPO) -> list:
    """How to supply `names` under the posture in force — the text every driver prints.
    A name of the form A|B|C (a role default that reads the first of several) is
    satisfied by any one of them; the first is the one to supply."""
    alts = [n.split("|") for n in names]
    extra = []
    for a in alts:
        if len(a) > 1:
            extra.append(f"{a[0]} is satisfied by any one of {', '.join(a)}")
        if set(a) & GITHUB_TOKEN_NAMES:
            extra.append("a GitHub token with contents:read on the release repository; for a one-off run "
                         "the GitHub CLI's own token serves: GH_TOKEN=$(gh auth token) <this same command>")
    return _hint([a[0] for a in alts], repo) + sorted(set(extra))


# Names a role reads a GitHub release-reading token from (goat-get, goat-docs).
GITHUB_TOKEN_NAMES = {"GOAT_CLI_RELEASE_TOKEN", "GH_TOKEN", "GHCR_READ_TOKEN"}


def _hint(names: list, repo: str) -> list:
    s = store(repo)
    joined = " ".join(names)
    rel = os.path.relpath(store_file(repo), repo) if store_file(repo) else ""
    if s == "1password":
        return [
            "put a reference line for each name (NAME=op://<vault>/<item>/<field>) in .envrc.local.d/<file>.refs"
            + " (goat's are in the goat-credentials repository, refs/goat.refs);",
            "sign in: eval $(op signin); then run: ops/scripts/with-secrets <this same command>",
        ]
    if s == "openbao":
        return [
            "put a reference line for each name (NAME=bao://<mount>/<path>/<field>) in .envrc.local.d/<file>.refs;",
            "sign in: bao login -method=oidc; then run: ops/scripts/with-secrets <this same command>",
        ]
    if s == "vaultwarden":
        return [
            "put a reference line for each name (NAME=bw://<collection>/<item>/<field>) in .envrc.local.d/<file>.refs;",
            "unlock: export BW_SESSION=\"$(bw unlock --raw)\"; then run: ops/scripts/with-secrets <this same command>",
        ]
    if s == "custom":
        return [f"supply references for {joined} the way $GOAT_SECRET_RUN expects, then run through ops/scripts/with-secrets"]
    if s == "env-file":
        return [
            f"add {joined} to {rel} (gitignored, mode 0600; NAME=value per line),",
            "or create the whole file with strong random values: goatbuilder secrets init --env <goatnet>",
        ]
    if s == "git-crypt":
        return [f"add {joined} to {rel} (encrypted in git by git-crypt) and run git-crypt unlock on this laptop"]
    if s == "sops":
        return [f"add {joined} to {rel} with: sops --input-type dotenv --output-type dotenv {rel}"]
    return [f"export {joined} in this shell before the command (the env store resolves nothing)"]


def doctor(repo: str = REPO) -> list:
    """(ok, check, detail, fix) rows for the posture in force."""
    rows = []
    try:
        s = store(repo)
    except PostureError as err:
        return [(False, "secrets:posture", str(err), f"set secrets.store in ops/goatbuilder/goatbuilder.yaml or {ENV_OVERRIDE['store']}")]
    src = declared_source(repo)
    rows.append((True, "secrets:posture", f"store {s} (from the {src})", ""))
    path = store_file(repo)
    rel = os.path.relpath(path, repo) if path else ""
    if s in ("1password", "openbao", "vaultwarden"):
        # One grammar, scheme-dispatched: each backend whose references are in play is
        # checked the same way — installed, reachable, signed in, references well-formed.
        # No doctor reads every value: a read is attributed per value, and the run
        # resolves them anyway.
        if s == "1password" or _refs_in_env("op://"):
            rows += _op_rows()
        if s == "openbao" or _refs_in_env("bao://"):
            rows += _bao_rows(repo)
        if s == "vaultwarden" or _refs_in_env("bw://"):
            rows += _bw_rows(repo)
    elif s == "env-file":
        if not os.path.isfile(path):
            rows.append((False, "secrets:env-file", f"{rel} does not exist", "goatbuilder secrets init --env <goatnet>"))
        else:
            mode = stat.S_IMODE(os.stat(path).st_mode)
            if mode & 0o077:
                rows.append((False, "secrets:env-file-mode", f"{rel} is mode {oct(mode)}; readable beyond its owner", f"chmod 600 {rel}"))
            ignored = subprocess.run(["git", "-C", repo, "check-ignore", "-q", path]).returncode == 0
            if not ignored:
                rows.append((False, "secrets:env-file-ignored", f"{rel} is NOT gitignored — its values would be committed", f"add {rel} to .gitignore"))
            if not any(r[0] is False and r[1].startswith("secrets:env-file") for r in rows):
                rows.append((True, "secrets:env-file", f"{rel}: present, mode 0600, gitignored — plaintext on this disk by the team's choice", ""))
    elif s == "git-crypt":
        if not os.path.isfile(path):
            rows.append((False, "secrets:git-crypt", f"{rel} does not exist", "create it under a git-crypt filter in .gitattributes"))
        elif _is_git_crypt_locked(path):
            rows.append((False, "secrets:git-crypt", f"{rel} is still encrypted", "git-crypt unlock"))
        else:
            rows.append((True, "secrets:git-crypt", f"{rel}: unlocked", ""))
    elif s == "sops":
        if not shutil.which("sops"):
            rows.append((False, "secrets:sops", "the sops CLI is not installed", "install sops and the key its .sops.yaml names (age or a cloud KMS)"))
        if not os.path.isfile(path):
            rows.append((False, "secrets:sops-file", f"{rel} does not exist", f"sops --input-type dotenv --output-type dotenv {rel}"))
    return rows


def _refs_in_env(scheme: str) -> list:
    return sorted(k for k, v in os.environ.items() if v.startswith(scheme))


def _malformed(scheme: str) -> list:
    """Three parts after the scheme: vault/item/field or mount/path/field."""
    return sorted(k for k in _refs_in_env(scheme) if len([p for p in os.environ[k][len(scheme):].split("/") if p]) < 3)


def _op_rows() -> list:
    rows = []
    if not shutil.which("op"):
        return [(False, "secrets:op", "the 1Password CLI (op) is not installed", "install it, or declare another store (docs/design/pluggable-dependencies.md)")]
    acct, err = op_account_resolve()
    if err:
        rows.append((False, "secrets:op-account", err.split(" — ")[0], "set OP_ACCOUNT=<your account> (example.1password.com for the goat team store), or declare secrets.op_account in ops/goatbuilder/goatbuilder.yaml"))
    elif acct:
        src = "OP_ACCOUNT" if os.environ.get("OP_ACCOUNT") else "goatbuilder.yaml"
        rows.append((True, "secrets:op-account", f"pinned to {acct} (from {src})", ""))
    else:
        rows.append((True, "secrets:op-account", "one account configured — op cannot fan out", ""))
    # Asking who is signed in can prompt (biometric unlock) or wait (a session
    # this machine does not have). A caller that only wants to know what is
    # declared sets GOAT_STORE_PROBE=0 and gets no prompt at all.
    if os.environ.get("GOAT_STORE_PROBE") == "0":
        rows.append((True, "secrets:op-session", "not checked: whether a session is open was not read, so nothing prompted you", ""))
        bad = _malformed("op://")
        if bad:
            rows.append((False, "secrets:op-refs", f"incomplete op:// reference in {', '.join(bad)}: op run refuses the whole run",
                         f"complete it to op://<vault>/<item>/<field>, or leave it out of one run with env -u {bad[0]}"))
        return rows
    who_cmd = ["op", "whoami"] + (["--account", acct] if acct else [])
    r = subprocess.run(who_cmd, capture_output=True, text=True, timeout=20, stdin=subprocess.DEVNULL)
    if r.returncode != 0:
        rows.append((False, "secrets:op-session", f"not signed in: {' '.join((r.stderr or r.stdout).split())[:200]}", "sign in: eval $(op signin)"))
    else:
        who = next((l.split(":", 1)[1].strip() for l in r.stdout.splitlines() if l.lower().startswith("email")), "signed in")
        rows.append((True, "secrets:op-session", f"signed in as {who}", ""))
    bad = _malformed("op://")
    if bad:
        rows.append((False, "secrets:op-refs", f"incomplete op:// reference in {', '.join(bad)} — op run refuses the whole run on it",
                     f"env -u {bad[0]} … or complete it to op://<vault>/<item>/<field>"))
    return rows


def _bao_rows(repo: str) -> list:
    # The store's contract (ADR 1203): references bao://<mount>/<path>/<field> (mount
    # 'goat'); BAO_ADDR is the store's certificate-only public edge (or an admin tunnel to its private address); BAO_CACERT is
    # the instance's CA certificate; the token comes from `bao login -method=oidc`
    # (~/.vault-token) or BAO_TOKEN; machines use AppRole.
    rows = []
    if not os.path.isfile(os.path.join(repo, "ops/secrets/openbao/run")):
        rows.append((False, "secrets:openbao", "the openbao store's injector (ops/secrets/openbao/run) is not on this checkout yet (ADR 1203)", "use another store until it lands"))
    addr = os.environ.get("BAO_ADDR", "")
    if not addr:
        rows.append((False, "secrets:openbao-addr", "BAO_ADDR is not set", "export BAO_ADDR=https://store.team.<zone>:8443 (a card or YubiKey certificate completes the handshake) or an admin tunnel to the private address"))
    cacert = os.environ.get("BAO_CACERT", "")
    if not cacert:
        rows.append((False, "secrets:openbao-cacert", "BAO_CACERT is not set", "export BAO_CACERT=<path to the store instance's ca.crt>"))
    elif not os.path.isfile(cacert):
        rows.append((False, "secrets:openbao-cacert", f"BAO_CACERT names {cacert}, which does not exist", "point BAO_CACERT at the store instance's ca.crt"))
    if not (os.environ.get("BAO_TOKEN") or os.path.isfile(os.path.expanduser("~/.vault-token"))):
        rows.append((False, "secrets:openbao-session", "not signed in: no BAO_TOKEN and no ~/.vault-token", "sign in: bao login -method=oidc"))
    elif not shutil.which("bao"):
        rows.append((False, "secrets:openbao-cli", "the OpenBao CLI (bao) is not installed", "install the bao CLI"))
    elif addr and os.environ.get("GOAT_STORE_PROBE") == "0":
        # Reaching the store's edge can ask for a card or YubiKey certificate. A
        # caller that only wants what is declared sets GOAT_STORE_PROBE=0.
        rows.append((True, "secrets:openbao-session", "not checked: whether the token is valid was not read, so nothing prompted you", ""))
    elif addr:
        r = subprocess.run(["bao", "token", "lookup", "-format=json"], capture_output=True, text=True, timeout=20)
        if r.returncode != 0:
            rows.append((False, "secrets:openbao-session", f"bao token lookup failed: {' '.join((r.stderr or r.stdout).split())[:200]}",
                         "sign in: bao login -method=oidc (and check that BAO_ADDR answers: the edge needs a client certificate, the tunnel needs to be up)"))
        else:
            rows.append((True, "secrets:openbao-session", f"token valid at {addr}", ""))
    bad = _malformed("bao://")
    if bad:
        rows.append((False, "secrets:openbao-refs", f"incomplete bao:// reference in {', '.join(bad)} — the run refuses on it",
                     f"complete it to bao://<mount>/<path>/<field>"))
    return rows


def _bw_rows(repo: str) -> list:
    # The store's contract (ADR 1212): references bw://<collection>/<item>/<field>; the
    # Bitwarden CLI is signed in to the team's server (bw config server, bw login --apikey)
    # and unlocked in this shell (BW_SESSION from the person's own `bw unlock`); the
    # adapter never sees the master password and the server never sees a value.
    rows = []
    if not os.path.isfile(os.path.join(repo, "ops/secrets/vaultwarden/run")):
        rows.append((False, "secrets:vaultwarden", "the vaultwarden store's injector (ops/secrets/vaultwarden/run) is not on this checkout yet (ADR 1212)", "use another store until it lands"))
    bw = os.environ.get("GOAT_BW_CLI") or shutil.which("bw")
    if not bw:
        rows.append((False, "secrets:vaultwarden-cli", "the Bitwarden CLI (bw) is not installed", "brew install bitwarden-cli"))
    elif not os.environ.get("BW_SESSION"):
        rows.append((False, "secrets:vaultwarden-session", "not unlocked: BW_SESSION is unset", "export BW_SESSION=\"$(bw unlock --raw)\" (after bw config server <url> and bw login --apikey, once)"))
    elif os.environ.get("GOAT_STORE_PROBE") == "0":
        # Whether the vault is unlocked is a fact about a credential, and asking
        # reaches the CLI and the team's server. A caller that only wants to know
        # what is declared sets GOAT_STORE_PROBE=0, as it does for `op whoami`,
        # and the row says it was not read rather than passing.
        rows.append((True, "secrets:vaultwarden-session", "not checked: whether the vault is unlocked was not read", ""))
    else:
        r = subprocess.run([bw, "status", "--nointeraction"], capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL)
        try:
            status = json.loads(r.stdout or "{}")
        except ValueError:
            status = {}
        state = status.get("status", "")
        if r.returncode != 0 or state != "unlocked":
            rows.append((False, "secrets:vaultwarden-session", f"bw status is {state or 'unknown'}", "export BW_SESSION=\"$(bw unlock --raw)\"; if not logged in: bw login --apikey"))
        else:
            rows.append((True, "secrets:vaultwarden-session", f"unlocked at {status.get('serverUrl') or 'the configured server'}", ""))
    bad = _malformed("bw://")
    if bad:
        rows.append((False, "secrets:vaultwarden-refs", f"incomplete bw:// reference in {', '.join(bad)} — the run refuses on it",
                     "complete it to bw://<collection>/<item>/<field>"))
    rows.append(_bw_status_row(repo))
    return rows


def _bw_status_row(repo: str) -> tuple:
    """The shape's freshness, from ops/secrets/vaultwarden/STATUS.yaml: whether it is
    still WIP, when it was measured, and when a smoke last passed. Informational (ok)
    so a doctor run is not failed by it; the text is what a person needs to know."""
    path = os.path.join(repo, "ops/secrets/vaultwarden/STATUS.yaml")
    try:
        text = _read(path)
    except OSError:
        return (True, "secrets:vaultwarden-status", "no STATUS.yaml on this checkout", "")
    kv = {}
    for line in text.splitlines():
        m = re.match(r"^([a-z_]+):\s*(.*?)\s*(#.*)?$", line)
        if m and m.group(1) != "lifts_when":
            kv[m.group(1)] = m.group(2).strip()
    status = kv.get("status", "?")
    smoke = kv.get("last_smoke", "null")
    smoke_txt = f"last smoke passed {smoke}" if smoke not in ("", "null", "~") else "no smoke recorded yet (ops/secrets/vaultwarden/smoke.sh --record)"
    detail = f"{'WIP, not for production use' if status == 'wip' else status} — built {kv.get('built', '?')}, gates measured {kv.get('measured', '?')}, {smoke_txt}"
    return (True, "secrets:vaultwarden-status", detail, "")


# --------------------------------------------------------------------------- quickstart: init

EXTERNAL = re.compile(
    r"(CLOUDFLARE|(^|_)AWS(_|$)|_SES(_|$)|SMTP|GITHUB|GHCR|(^|_)PAT$|NETBIRD.*(TOKEN|PAT)|SERVICE_USER|"
    r"(^|_)API_TOKEN$|ROUTE53|(^|_)AZURE(_|$)|DIGITALOCEAN|HCLOUD|SLACK|PAGERDUTY|TWILIO)"
)


def bindings_for(goatnet: str, repo: str = REPO) -> dict:
    """ENV NAME -> one ansible var naming it, over every member group of the goatnet."""
    sys.path.insert(0, os.path.join(repo, "ops/scripts"))
    import importlib.util
    spec = importlib.util.spec_from_file_location("dsn", os.path.join(repo, "ops/scripts/deploy-secrets-needed.py"))
    dsn = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(dsn)
    members = goatnet_members(goatnet, repo)
    out: dict = {}
    for g in members:
        for var, env in dsn.env_bindings(repo, g).items():
            out.setdefault(env, var)
    return out


def goatnet_members(goatnet: str, repo: str = REPO) -> list:
    """Member groups of a goatnet from ops/pins/fleet-pins.yaml, or the group itself."""
    try:
        text = _read(os.path.join(repo, "ops/pins/fleet-pins.yaml"))
    except OSError:
        text = ""
    key = goatnet.replace("-", "_")
    m = re.search(rf"^  {re.escape(key)}:\n((?:    .*\n|\s*\n)+)", text, re.M)
    if m:
        mm = re.search(r"members:\s*\[([^\]]*)\]", m.group(1))
        if mm:
            return [x.strip() for x in mm.group(1).split(",") if x.strip()]
    for cand in (key, f"{key}_hosts"):
        if os.path.isdir(os.path.join(repo, "ops/ansible/group_vars", cand)):
            return [cand]
    if goatnet == lite_name(repo):
        raise PostureError(f"the goatlite {goatnet} has no config surface yet (ops/ansible/group_vars/{key}_hosts) — "
                           + _first_standup(goatnet))
    raise PostureError(f"no goatnet or group named '{goatnet}' (ops/pins/fleet-pins.yaml goatnets, or ops/ansible/group_vars/)")


def generated_value(name: str) -> str:
    # 32 hex characters: a valid oauth2-proxy cookie secret (32 bytes as text),
    # and Ory's kratos cipher key, which ops/ansible/ory-deploy.yml asserts is
    # exactly 32 characters because it is an AES key. Everything else is a
    # strong password, where longer is better. A 43-character cipher key failed
    # that assertion with "Missing or undersized Ory secrets", whose recovery
    # paths both assume a cluster of sites a goatlite does not have.
    if "COOKIE" in name or "CIPHER" in name:
        return _secrets.token_hex(16)
    # A network id is a UUID: Ory's root tenant row is inserted into a uuid
    # column, and a 43-character token made Postgres refuse the insert with
    # "invalid input syntax for type uuid". The role's own default is a UUID.
    if name.endswith("_NID") or "UUID" in name:
        return str(_uuid.uuid4())
    return _secrets.token_urlsafe(32)


def init(goatnet: str, path: str, force: bool, repo: str = REPO, if_env_file: bool = False) -> int:
    """Write (or extend) a 0600 env file with a strong random value for every variable the
    goatnet's plays template. Values already in the file are kept — a re-run never rotates.
    `if_env_file` (a goatlite's phase 1): under any store but env-file there is no such
    file to create, so say so and change nothing."""
    if if_env_file and store(repo) != "env-file":
        print(f"secrets init: the store in force is {store(repo)}, not env-file — nothing to create here")
        return 0
    names = bindings_for(goatnet, repo)
    if not names:
        print(f"secrets init: the {goatnet} groups template no variables from the environment")
        return 0
    rel = os.path.relpath(path, repo)
    if os.path.exists(path) and not os.path.isfile(path):
        raise PostureError(f"{rel} exists and is not a file")
    ignored = subprocess.run(["git", "-C", repo, "check-ignore", "-q", path]).returncode == 0
    if not ignored and not force:
        raise PostureError(f"refusing to write values to {rel}: it is not gitignored (add it to .gitignore, or pass --force)")
    existing = parse_dotenv(_read(path)) if os.path.isfile(path) else {}
    generated, external, kept = [], [], []
    lines = [
        f"# goat secrets for {goatnet}: VALUES IN PLAINTEXT, by this team's declared choice (ADR 1204).",
        "# Gitignored and mode 0600. Never commit, copy or paste this file. Regenerating keeps every value below.",
        "",
    ]
    for env in sorted(names):
        if existing.get(env):
            kept.append(env)
            lines.append(f"{env}={existing[env]}")
        elif EXTERNAL.search(env):
            external.append(env)
            lines.append(f"# EXTERNAL: a value issued by an outside service, or empty when the quickstart bypasses it")
            lines.append(f"{env}=")
        else:
            generated.append(env)
            lines.append(f"{env}={generated_value(env)}")
    for env, value in sorted(existing.items()):  # keep anything the operator added by hand
        if env not in names:
            lines.append(f"{env}={value}")
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path) or ".", prefix=".secrets-init-")
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write("\n".join(lines) + "\n")
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise
    os.chmod(path, 0o600)
    print(f"secrets init: {rel} (mode 0600, gitignored) for {goatnet}: "
          f"{len(generated)} generated, {len(kept)} kept, {len(external)} need an outside value")
    for env in external:
        print(f"  EXTERNAL  {env}  (issued by an outside service; see docs/design/pluggable-dependencies.md §Quickstart)")
    print(f"declare the posture: secrets: {{store: env-file}} in ops/goatbuilder/goatbuilder.yaml, or export GOAT_SECRET_STORE=env-file")
    return 0


# --------------------------------------------------------------------------- value stores: set

def _write_dotenv(path: str, lines: list) -> None:
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path) or ".", prefix=".secrets-set-")
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write("\n".join(lines) + "\n")
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise
    os.chmod(path, 0o600)


def _sops_reencrypt(path: str, plain_lines: list) -> None:
    """Encrypt a dotenv body in place of `path` with the creation rule that path matches.
    The plaintext goes through a 0600 file in the same directory and never through argv."""
    if not shutil.which("sops"):
        raise PostureError("sops store: the sops CLI is not installed")
    d = os.path.dirname(path) or "."
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".secrets-set-", suffix=".env")
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write("\n".join(plain_lines) + "\n")
        r = subprocess.run(["sops", "--encrypt", "--input-type", "dotenv", "--output-type", "dotenv",
                            "--filename-override", path, tmp], capture_output=True, text=True)
        if r.returncode != 0:
            raise PostureError(f"sops store: could not encrypt {path}: {r.stderr.strip()[:300]}")
        _write_dotenv(path, r.stdout.rstrip("\n").splitlines())
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def set_values(new: dict, repo: str = REPO) -> dict:
    """Store `new` ({NAME: value}) under the value store in force. A name that already
    holds a value is KEPT (never rotated); the rest are written. Returns
    {NAME: 'kept'|'written'}. Refuses under a reference store (it takes references,
    not values) and under `env` (nothing persists). Values are never printed."""
    s = store(repo)
    bad = [n for n in new if not NAME.match(n)]
    if bad:
        raise PostureError(f"set: not a variable name: {', '.join(sorted(bad))}")
    if s in REFERENCE_STORES:
        raise PostureError(f"set: the {s} store holds references, not values — write each value into the "
                           f"store with its own tooling and put NAME={DEFAULT_PREFIX.get(s, '<scheme>://')}... "
                           "in .envrc.local.d/<file>.refs")
    if s == "env":
        raise PostureError("set: the env store persists nothing — export each NAME=value in this shell "
                           "(or declare a store: docs/design/pluggable-dependencies.md)")
    path = store_file(repo)
    if s == "sops":
        current = values(repo) if os.path.isfile(path) else {}
    elif os.path.isfile(path):
        if s == "git-crypt" and _is_git_crypt_locked(path):
            raise PostureError(f"git-crypt store: {path} is still encrypted — run: git-crypt unlock")
        current = parse_dotenv(_read(path))
    else:
        current = {}
    inside = os.path.commonpath([os.path.realpath(path), os.path.realpath(repo)]) == os.path.realpath(repo)
    if s == "env-file" and not os.path.isfile(path) and inside:
        ignored = subprocess.run(["git", "-C", repo, "check-ignore", "-q", path]).returncode == 0
        if not ignored:
            rel = os.path.relpath(path, repo)
            raise PostureError(f"refusing to write values to {rel}: it is not gitignored (add it to .gitignore)")
    result = {}
    lines = _read(path).rstrip("\n").splitlines() if (os.path.isfile(path) and s != "sops") else \
        [f"{k}={v}" for k, v in current.items()]
    if not lines and s != "sops":
        lines = [f"# goat secrets: VALUES IN PLAINTEXT, by this team's declared choice (ADR 1204).",
                 "# Gitignored and mode 0600. Never commit, copy or paste this file.", ""]
    for name in sorted(new):
        if current.get(name):
            result[name] = "kept"
            continue
        result[name] = "written"
        if name in current:  # present but empty: fill the line in place
            lines = [f"{name}={new[name]}" if re.match(rf"^(export\s+)?{re.escape(name)}\s*=", ln.strip()) else ln
                     for ln in lines]
        else:
            lines.append(f"{name}={new[name]}")
    if any(v == "written" for v in result.values()):
        if s == "sops":
            _sops_reencrypt(path, lines)
        else:
            _write_dotenv(path, lines)
    return result


# --------------------------------------------------------------------------- cli

def main(argv: list) -> int:
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0 if argv else 2
    cmd, rest = argv[0], argv[1:]
    try:
        if cmd == "store":
            print(store())
        elif cmd == "kind":
            # `reference` when the store keeps pointers here and fetches the
            # values per command, `value` when it keeps the values themselves.
            # This module owns the list (REFERENCE_STORES), so every caller --
            # the shell wrapper and the binary's doctor -- reads it from here
            # rather than keeping a copy that goes stale when a store is added.
            s = store()
            print("reference" if s in REFERENCE_STORES else "value")
        elif cmd == "available":
            print("\n".join(sorted(available())))
        elif cmd == "missing":
            have = available()
            miss = [n for n in rest if n not in have]
            if miss:
                print(" ".join(miss))
            return 1 if miss else 0
        elif cmd == "hint":
            print("\n".join(hint(rest)))
        elif cmd == "op-account":
            # The account to pin before an `op` call. Prints it on stdout when one is
            # resolved; prints the refusal on stderr and exits 3 when OP_ACCOUNT is
            # unset and more than one account is configured (with-secrets relays it).
            acct, err = op_account_resolve()
            if err:
                print(f"secrets: {err}", file=sys.stderr)
                return 3
            if acct:
                print(acct)
            return 0
        elif cmd == "doctor":
            bad = 0
            for ok, check, detail, fix in doctor():
                print(f"{'ok  ' if ok else 'FAIL'}  {check}  {detail}" + (f"  ||  fix: {fix}" if fix else ""))
                bad += not ok
            return 1 if bad else 0
        elif cmd == "exec":
            if rest and rest[0] == "--":
                rest = rest[1:]
            if not rest:
                print("usage: posture.py exec -- CMD...", file=sys.stderr)
                return 2
            env = dict(os.environ)
            env.update(values())
            env["GOAT_SECRETS_RESOLVED"] = "1"
            os.execvpe(rest[0], rest, env)
        elif cmd == "init":
            goatnet, path, force, if_env_file = "", "", False, False
            i = 0
            while i < len(rest):
                if rest[i] == "--if-env-file":
                    if_env_file = True; i += 1
                elif rest[i] == "--env":
                    goatnet = rest[i + 1]; i += 2
                elif rest[i] == "--file":
                    path = rest[i + 1]; i += 2
                elif rest[i] == "--force":
                    force = True; i += 1
                else:
                    print(f"posture.py init: unknown argument {rest[i]}", file=sys.stderr)
                    return 2
            if not goatnet:
                print("usage: posture.py init --env GOATNET [--file PATH] [--force] [--if-env-file]", file=sys.stderr)
                return 2
            target = path or setting("env_file")
            target = target if os.path.isabs(target) else os.path.join(REPO, target)
            return init(goatnet, target, force, if_env_file=if_env_file)
        elif cmd == "set":
            if rest != ["--stdin-json"]:
                print("usage: posture.py set --stdin-json   (a JSON object {NAME: value} on stdin)", file=sys.stderr)
                return 2
            try:
                payload = json.load(sys.stdin)
            except ValueError:
                print("secrets: set: stdin is not a JSON object", file=sys.stderr)
                return 2
            if not isinstance(payload, dict) or not all(isinstance(v, str) for v in payload.values()):
                print("secrets: set: want a JSON object of NAME: string", file=sys.stderr)
                return 2
            where = os.path.relpath(store_file(), REPO)
            for name, state in sorted(set_values(payload).items()):
                print(f"{name}\t{state}\t{where}")
        else:
            print(f"posture.py: unknown command {cmd}", file=sys.stderr)
            return 2
    except PostureError as err:
        print(f"secrets: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
