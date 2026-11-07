#!/usr/bin/env python3
"""Persist newsite-secrets' live SPEC without plaintext files or secret argv.

The store is the declared secrets posture (ops/secrets/posture.py, ADR 1204):
  1password        receives a JSON item over stdin; references op:// are written
  openbao          ops/secrets/openbao/bao-init receives {site, secrets}; bao:// references
  custom           $GOAT_SECRET_INIT receives {site, secrets: {NAME: value}} over stdin
                   and returns {NAME: reference}
  env-file, sops, git-crypt
                   the same SPEC of names, with values written into the posture's
                   value file through posture.set_values (never printed, never
                   rotated); no reference file is touched, the store IS the file
  env              refused: nothing persists under the env store
Every store must create missing values without overwriting existing values on
retries.
"""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import tempfile
import uuid


class Failure(Exception):
    pass


_POSTURE = None
# posture.py's own tables, repeated here only for a checkout that lacks the file
# (the generator's fixture tests copy these two scripts alone). With posture.py
# present its tables are authoritative.
STORES = ("1password", "openbao", "custom", "sops", "git-crypt", "env-file", "env")
VALUE_STORES = ("sops", "git-crypt", "env-file")


def posture():
    """ops/secrets/posture.py as a module (the one place every caller asks about the
    store), or None when this checkout has no posture.py — then only GOAT_SECRET_STORE
    in the environment decides, as before the posture existed."""
    global _POSTURE
    if _POSTURE is None:
        path = Path(__file__).resolve().parent.parent / "secrets" / "posture.py"
        if not path.is_file():
            _POSTURE = False
        else:
            spec = importlib.util.spec_from_file_location("goat_posture", path)
            mod = importlib.util.module_from_spec(spec)
            try:
                spec.loader.exec_module(mod)
            except (OSError, ImportError):
                raise Failure(f"cannot load the secrets posture ({path})") from None
            _POSTURE = mod
    return _POSTURE or None


def store_name():
    """The store in force. GOAT_SECRET_STORE in the environment wins (posture.py's own
    first rule); the declared posture is read from the repository this script sits in."""
    p = posture()
    if p is None:
        return os.environ.get("GOAT_SECRET_STORE", "1password")
    try:
        return p.store(str(Path(__file__).resolve().parents[2]))
    except p.PostureError as error:
        raise Failure(str(error)) from None


def value_stores():
    p = posture()
    return p.VALUE_STORES if p else VALUE_STORES


def all_stores():
    p = posture()
    return p.STORES if p else STORES


def spec_from_lines(lines):
    spec = {}
    for line in lines:
        name, kind, _comment = line.rstrip("\n").split("|", 2)
        if not re.fullmatch(r"[A-Z][A-Z0-9_]*", name) or name in spec:
            raise Failure("invalid or duplicate SPEC name")
        if kind not in {"hex32", "hex40", "hex48", "b64_32", "b64_36", "uuid"} and not kind.startswith("literal:") and not kind.startswith("pair:"):
            raise Failure(f"unknown format for {name}")
        spec[name] = kind
    if not spec:
        raise Failure("empty credential SPEC")
    return spec


def generate(kind):
    if kind.startswith("pair:"):
        handle = kind.partition(":")[2]
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.@-]{0,63}", handle):
            raise Failure("pair generation requires a valid explicit bootstrap handle")
        return handle + ":" + base64.b64encode(secrets.token_bytes(32)).decode()
    if kind.startswith("literal:"):
        return kind.partition(":")[2]
    if kind.startswith("hex"):
        return secrets.token_hex(int(kind[3:]) // 2)
    if kind.startswith("b64_"):
        return base64.b64encode(secrets.token_bytes(int(kind[4:]))).decode()
    return str(uuid.uuid4())


def run_json(argv, payload=None):
    # Provider stderr and exceptions may echo values; never forward either.
    try:
        result = subprocess.run(argv, input=None if payload is None else json.dumps(payload),
                                text=True, capture_output=True, check=True)
        return json.loads(result.stdout)
    except (OSError, subprocess.CalledProcessError, ValueError):
        raise Failure("secret-store command failed; no references written; check store access") from None


def reference_ok(value, prefix):
    # dotenv accepts quoted references, but shell metacharacters must never
    # enter a generated assignment. Provider IDs and field labels fit this set.
    return isinstance(value, str) and value.startswith(prefix) and len(value) > len(prefix) and bool(
        re.fullmatch(r"[A-Za-z0-9:/_.@%+\-]+", value))


def assignments(path, prefix=None):
    """Read names only from legacy files; strictly validate reference files."""
    names = set()
    if not path.exists():
        return names
    if not path.is_file() or path.is_symlink():
        raise Failure("credential path must be a regular file, not a symlink")
    with path.open() as stream:
        for line in stream:
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            match = re.match(r"\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=", line)
            if prefix is not None:
                ref = re.fullmatch(r'''\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=(['"]?)([A-Za-z0-9:/_.@%+\-]+)\2\s*(?:#.*)?\n?''', line)
                if not ref or not reference_ok(ref[3], prefix) or ref[1] in names:
                    raise Failure("invalid or duplicate assignment in reference file; no values printed")
                match = ref
            if match:
                names.add(match[1])
    return names


def onepassword(site, spec, vault):
    if not vault:
        raise Failure("set GOAT_SECRET_VAULT or pass --vault; no vault is assumed")
    vault_id = run_json(["op", "vault", "get", vault, "--format", "json"])["id"]
    title = f"goat site {site} services"
    items = run_json(["op", "item", "list", "--vault", vault_id, "--format", "json"])
    matches = [item for item in items if item.get("title") == title]
    if len(matches) > 1:
        raise Failure("duplicate site-secret items; select and consolidate them before retrying")
    if matches:
        item = run_json(["op", "item", "get", matches[0]["id"], "--vault", vault_id, "--format", "json"])
        if item.get("category") != "SECURE_NOTE" or "goat-newsite-secrets" not in item.get("tags", []):
            raise Failure("existing item is not managed by newsite-secrets; refusing to edit it")
    else:
        item = {"title": title, "category": "SECURE_NOTE", "tags": ["goat-newsite-secrets"], "fields": []}
    labels = {}
    for field in item["fields"]:
        label = field.get("label")
        if label in spec:
            if label in labels or not field.get("value") or field.get("section"):
                raise Failure("ambiguous or empty stored field; refusing to rotate it")
            if spec[label].startswith("pair:"):
                parts = field["value"].split(":")
                if len(parts) != 2 or parts[0] != spec[label].partition(":")[2] or not parts[1].strip():
                    raise Failure("stored bootstrap pair is malformed or belongs to a different handle; refusing to replace it")
            labels[label] = field
    missing = {name: kind for name, kind in spec.items() if name not in labels}
    for name, kind in missing.items():
        item["fields"].append({"label": name, "type": "CONCEALED", "value": generate(kind)})
    if missing:
        if matches:
            item = run_json(["op", "item", "edit", item["id"], "--vault", vault_id, "--format", "json"], item)
        else:
            item = run_json(["op", "item", "create", "--vault", vault_id, "--format", "json", "-"], item)
    # Confirm the returned item contains each field before writing its pointer.
    actual = [f.get("label") for f in item.get("fields", []) if f.get("value") and not f.get("section")]
    if any(actual.count(name) != 1 for name in spec):
        raise Failure("store did not return every requested field; retry recovers stored values")
    return {name: f"op://{vault_id}/{item['id']}/{name}" for name in spec}


def persist_values(spec):
    """Value stores (env-file, sops, git-crypt): generate each missing value and hand the
    set to posture.set_values in-process, so no value crosses argv, a pipe or stdout.
    Returns {NAME: 'kept'|'written'}."""
    p = posture()
    if p is None:
        raise Failure("a value store needs ops/secrets/posture.py in this checkout")
    try:
        return p.set_values({name: generate(kind) for name, kind in spec.items()},
                            str(Path(__file__).resolve().parents[2]))
    except p.PostureError as error:
        raise Failure(str(error)) from None


def persist(site, spec, vault, prefix):
    store = store_name()
    if store in value_stores():
        return persist_values(spec)
    if store == "env":
        raise Failure("the env store persists nothing: export each SPEC name in this shell, or declare a "
                      "store (secrets.store in goatbuilder.yaml; docs/design/pluggable-dependencies.md)")
    if store == "1password":
        if prefix != "op://":
            raise Failure("1password requires GOAT_SECRET_REF_PREFIX=op://")
        return onepassword(site, spec, vault)
    if store == "openbao":
        if prefix != "bao://":
            raise Failure("openbao requires GOAT_SECRET_REF_PREFIX=bao://")
        writer = str(Path(__file__).resolve().parent.parent / "secrets" / "openbao" / "bao-init")
        return run_json([writer], {"site": site, "secrets": {name: generate(kind) for name, kind in spec.items()}})
    if store == "custom":
        writer = os.environ.get("GOAT_SECRET_INIT")
        if not writer:
            raise Failure("custom store needs GOAT_SECRET_INIT pointing to an executable adapter")
        return run_json([writer], {"site": site, "secrets": {name: generate(kind) for name, kind in spec.items()}})
    raise Failure(f"unknown secrets store {store!r} — one of: {', '.join(all_stores())} "
                  "(GOAT_SECRET_STORE, or secrets.store in goatbuilder.yaml)")


def trim_portal_roster(spec, present, handle):
    """Never add a second portal authority or replace a named roster on a site that
    already has a portal seed; without an explicit handle, mint no bootstrap pair."""
    portal_names = {name for name in spec if name.endswith(("_PORTAL_OPERATOR_API_KEY", "_PORTAL_OPERATOR_API_KEYS"))}
    if portal_names & present:
        if handle:
            print("existing portal roster retained; bootstrap option makes no operator grant; use the audited platform-admin workflow", file=sys.stderr)
        return {name: kind for name, kind in spec.items() if name not in portal_names}
    if not handle:
        return {name: kind for name, kind in spec.items() if not kind.startswith("pair:")}
    return spec


def main_values(args, spec, handle, store):
    """The value-store path (env-file, sops, git-crypt): the posture's file is the store.
    Presence is what posture.available() reports — the file's names plus the shell's — so
    a re-run never rotates; the missing names are generated and written through
    posture.set_values. No reference file is involved."""
    p = posture()
    if p is None:
        raise Failure(f"the {store} store needs ops/secrets/posture.py in this checkout")
    repo = str(Path(__file__).resolve().parents[2])
    if args.file:
        raise Failure(f"--file names a reference file; the {store} store writes values to its own file "
                      f"({os.path.relpath(p.store_file(repo), repo) if p.store_file(repo) else 'none'})")
    try:
        have = p.available(repo)
    except p.PostureError as error:
        raise Failure(str(error)) from None
    present = {name for name in spec if name in have}
    spec = trim_portal_roster(spec, present, handle)
    missing = {name: kind for name, kind in spec.items() if name not in present}
    if args.dry_run:
        for name in missing:
            print(f"would generate: {name}")
        print(f"dry-run ({store} store): {len(missing)} to generate, {len(spec) - len(missing)} already present")
        return
    if not missing:
        print("all site credentials already present; nothing to do")
        return
    if store == "env":
        raise Failure(f"the env store persists nothing: {len(missing)} value(s) would be lost. Export each of "
                      f"{' '.join(sorted(missing))} in this shell, or declare a store "
                      "(secrets.store in goatbuilder.yaml; docs/design/pluggable-dependencies.md)")
    path = Path(p.store_file(repo))
    lock = path.with_name(path.name + ".lock")
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        lock.mkdir(mode=0o700)
    except FileExistsError:
        raise Failure("the secrets file is locked by another generator; no write attempted") from None
    try:
        result = persist_values(missing)
    finally:
        lock.rmdir()
    written = sum(1 for state in result.values() if state == "written")
    kept = len(result) - written
    rel = os.path.relpath(path, repo)
    print(f"wrote {written} value(s) to {rel} under the {store} store "
          f"({kept} kept, {len(spec) - len(missing)} already present); no value was printed")
    print("next: run credential consumers through ops/scripts/with-secrets (the posture resolves the file per command)")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("site")
    parser.add_argument("--file", type=Path)
    parser.add_argument("--vault", default=os.environ.get("GOAT_SECRET_VAULT"))
    parser.add_argument("--bootstrap-operator", help="explicit named operator for the initial portal seed only")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true")
    mode.add_argument("--spec-json", action="store_true")
    args = parser.parse_args()
    spec = spec_from_lines(sys.stdin)
    handle = args.bootstrap_operator
    if handle is not None and (not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.@-]{0,63}", handle) or handle == "operator"):
        raise Failure("bootstrap operator must be an explicit non-reserved handle (letters, digits, dot, @, underscore, hyphen)")
    if handle:
        spec = {name: "pair:" + handle if kind == "pair:" else kind for name, kind in spec.items()}
    if args.spec_json:
        print(json.dumps(spec, sort_keys=True))
        return
    store = store_name()
    if store in value_stores() or store == "env":
        main_values(args, spec, handle, store)
        return
    common = subprocess.check_output(["git", "-C", str(Path(__file__).parent), "rev-parse",
                                      "--path-format=absolute", "--git-common-dir"], text=True).strip()
    root = Path(common).parent
    refs_dir = root / ".envrc.local.d"
    if store == "openbao":
        refs_dir = refs_dir / "openbao"       # the green reference tree (ADR 1203)
    target = args.file or refs_dir / "30-site-services.refs"
    if target.suffix != ".refs":
        raise Failure("--file must name a .refs file; plaintext output is no longer supported")
    prefix = os.environ.get("GOAT_SECRET_REF_PREFIX", "bao://" if store == "openbao" else "op://")
    if not reference_ok(prefix + "test", prefix):
        raise Failure("invalid GOAT_SECRET_REF_PREFIX")
    if not args.dry_run and (not target.parent.is_dir() or target.parent.is_symlink()):
        raise Failure("reference directory must already exist and must not be a symlink")
    lock = target.with_name(target.name + ".lock")
    locked = False
    try:
        if not args.dry_run:
            try:
                lock.mkdir(mode=0o700)
                locked = True
            except FileExistsError:
                raise Failure("reference file is locked by another generator; no write attempted") from None
        present = assignments(target, prefix)
        # Detect existing credentials across the loader's sources, without
        # sourcing shell code or displaying values. Do not silently re-key a
        # legacy site or a site whose references live in a different file.
        paths = [root / ".envrc.local"]
        paths += list((root / ".envrc.local.d").glob("*.envrc"))
        paths += list((root / ".envrc.local.d").glob("*.refs"))
        for path in paths:
            if path != target:
                present |= assignments(path, prefix if path.suffix == ".refs" else None)
        # Existing values in the command environment also belong to the site.
        present |= {name for name in spec if os.environ.get(name)}
        spec = trim_portal_roster(spec, present, handle)
        missing = {name: kind for name, kind in spec.items() if name not in present}
        if args.dry_run:
            for name in missing:
                print(f"would generate: {name}")
            print(f"dry-run: {len(missing)} to generate, {len(spec) - len(missing)} already present")
            return
        if not missing:
            print("all site credentials already present; nothing to do")
            return
        refs = persist(args.site, missing, args.vault, prefix)
        if not isinstance(refs, dict) or set(refs) != set(missing) or not all(reference_ok(ref, prefix) for ref in refs.values()):
            raise Failure("store returned invalid references; no references written")
        # Replace the complete reference file atomically. The scratch file
        # contains references only; an interrupted write cannot leave a partial
        # assignment. A provider retry reuses fields if this step fails.
        previous = target.read_text() if target.exists() else ""
        fd, scratch = tempfile.mkstemp(prefix=".newsite-refs-", dir=target.parent)
        try:
            with os.fdopen(fd, "w") as stream:
                stream.write(previous)
                stream.write(f"\n# {args.site} site credentials (newsite-secrets; ADR 1186)\n")
                for name, ref in refs.items():
                    stream.write(f'export {name}="{ref}"\n')
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(scratch, target)
        finally:
            if os.path.exists(scratch):
                os.unlink(scratch)
        print(f"appended {len(refs)} reference(s) to {target} ({len(spec) - len(missing)} already present)")
        print("next: direnv allow; run credential consumers through ops/scripts/with-secrets")
    finally:
        if locked:
            lock.rmdir()


if __name__ == "__main__":
    try:
        main()
    except Failure as error:
        print(f"newsite-secrets: {error}", file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, subprocess.SubprocessError):
        # Invalid provider output and OS errors must not print a secret-bearing
        # traceback. No implicit fallback to local plaintext.
        print("newsite-secrets: invalid store response or local I/O failure; no plaintext fallback", file=sys.stderr)
        sys.exit(1)
