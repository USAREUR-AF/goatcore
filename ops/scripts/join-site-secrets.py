#!/usr/bin/env python3
"""join-site-secrets.py — a joining site's secrets: the shared ones taken from its founder,
the rest minted (docs/design/join-node-site-secrets.md).

    join-site-secrets.py <site> --founder <founder> [--dry-run]

The names every site shares are the SPEC lines of newsite-secrets.sh marked `cluster`
(`newsite-secrets.sh <site> --cluster-json`). For each, <SITE>_X is made to resolve to
<FOUNDER>_X:

  a store of references (1password, openbao, vaultwarden, custom): <SITE>_X=<the founder's
      reference> is appended to the reference file newsite-secrets.sh writes; no value is
      read or copied
  a store of values (env-file, sops, git-crypt): the founder's value is written under the
      joiner's name with posture.set_values; never printed
  env: nothing persists; the joiner's names must already be exported

A joiner name that already holds something is never overwritten. Then newsite-secrets.sh
mints the site's own values (it skips every name already present), and, under
ops/scripts/with-secrets, each shared name's resolved value at the joiner is compared with
the founder's by SHA-256: one line per name, `same` or `differs`, never a value or digest.

Refusals (exit 1), each naming variables, never values:
  GB-352  the founder's value for a shared name is not there to take, or does not resolve
  GB-353  the joiner already holds a different value for a shared name
  GB-354  the env store holds nothing, and a joiner's shared name is not exported
--dry-run prints what it would do and writes nothing.
"""
import argparse
import hashlib
import hmac
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
FOUNDER_MISSING, JOINER_DIFFERS, ENV_STORE = "GB-352", "GB-353", "GB-354"
SITE_RE = re.compile(r"[a-z0-9][a-z0-9-]*")
REF_LINE = re.compile(r'''\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=(['"]?)([A-Za-z0-9:/_.@%+\-]+)\2\s*(?:#.*)?''')
SCHEMES = ("op://", "bao://", "bw://")


class Refusal(Exception):
    def __init__(self, code, text):
        super().__init__(f"{code}: {text}")


class Failure(Exception):
    pass


def upper(site):
    return site.upper().replace("-", "_")


def posture():
    spec = importlib.util.spec_from_file_location("goat_posture", REPO / "ops/secrets/posture.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def shared_suffixes(site):
    """The shared names, without the site prefix, from the one SPEC."""
    out = subprocess.run(["bash", str(HERE / "newsite-secrets.sh"), site, "--cluster-json"],
                         capture_output=True, text=True)
    if out.returncode != 0:
        raise Failure("newsite-secrets.sh --cluster-json failed: " + out.stderr.strip()[:300])
    pre = upper(site) + "_"
    names = json.loads(out.stdout)
    if not names or not all(n.startswith(pre) for n in names):
        raise Failure("newsite-secrets.sh listed no shared names, or names of another site")
    return [n[len(pre):] for n in names]


def unresolved(value, prefix):
    return not value or any(value.startswith(s) for s in SCHEMES + ((prefix,) if prefix else ()))


def refs_dir(store):
    common = subprocess.check_output(["git", "-C", str(HERE), "rev-parse", "--path-format=absolute",
                                      "--git-common-dir"], text=True).strip()
    d = Path(common).parent / ".envrc.local.d"
    return d / "openbao" if store == "openbao" else d


def read_refs(directory):
    """Every NAME=reference of the directory's .refs files; a malformed file is refused
    without printing it."""
    refs = {}
    for path in sorted(directory.glob("*.refs")) if directory.is_dir() else []:
        if path.is_symlink() or not path.is_file():
            raise Failure(f"{path.name}: a reference file must be a regular file")
        for line in path.read_text().splitlines():
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            m = REF_LINE.fullmatch(line)
            if not m:
                raise Failure(f"{path.name}: a line is not NAME=reference; nothing printed")
            refs.setdefault(m[1], m[3])
    return refs


def append_refs(target, site, founder, pairs):
    """Append NAME=reference lines to target atomically, under the generator's lock."""
    if not target.parent.is_dir() or target.parent.is_symlink():
        raise Failure("the reference directory must already exist and must not be a symlink")
    lock = target.with_name(target.name + ".lock")
    try:
        lock.mkdir(mode=0o700)
    except FileExistsError:
        raise Failure("the reference file is locked by another writer; nothing written") from None
    try:
        previous = target.read_text() if target.exists() else ""
        fd, scratch = tempfile.mkstemp(prefix=".join-site-refs-", dir=target.parent)
        try:
            with os.fdopen(fd, "w") as stream:
                stream.write(previous)
                stream.write(f"\n# {site}: shared with its founder {founder} (join-site-secrets; ADR 1186)\n")
                for name, ref in pairs:
                    stream.write(f'export {name}="{ref}"\n')
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(scratch, 0o600)
            os.replace(scratch, target)
        finally:
            if os.path.exists(scratch):
                os.unlink(scratch)
    finally:
        lock.rmdir()


def take(site, founder, suffixes, store, dry_run):
    """Step 1. Returns {NAME: reference} for the child that compares (reference stores)."""
    s, f = upper(site), upper(founder)
    prefix = os.environ.get("GOAT_SECRET_REF_PREFIX", "bao://" if store == "openbao" else "op://")
    p = posture()
    if store == "env":
        missing = [f"{s}_{x}" for x in suffixes if not os.environ.get(f"{s}_{x}")]
        if missing:
            raise Refusal(ENV_STORE, f"the env store holds nothing, and {', '.join(missing)} "
                          "are not exported; export each with its founder's value, or declare a store "
                          "(secrets.store in goatbuilder.yaml)")
        print(f"env store: every shared name of {site} is exported; the comparison decides")
        return {}
    if store in p.VALUE_STORES:
        try:
            vals = p.values(str(REPO))
        except p.PostureError as error:
            raise Failure(str(error)) from None
        new = {}
        for x in suffixes:
            fv = os.environ.get(f"{f}_{x}") or vals.get(f"{f}_{x}")
            sv = os.environ.get(f"{s}_{x}") or vals.get(f"{s}_{x}")
            if not fv:
                raise Refusal(FOUNDER_MISSING, f"{f}_{x} has no value under the {store} store, so {s}_{x} "
                              f"cannot take it: store it (python3 ops/secrets/posture.py hint {f}_{x}), then resume")
            if sv and not hmac.compare_digest(sv.encode(), fv.encode()):
                raise Refusal(JOINER_DIFFERS, f"{s}_{x} already holds a value that is not {f}_{x}'s; "
                              "it is not overwritten: decide which is right and remove the other")
            if sv:
                print(f"{s}_{x}: already holds {f}_{x}'s value")
            else:
                print(f"{s}_{x} takes {f}_{x}'s value ({store} store)")
                new[f"{s}_{x}"] = fv
        if new and not dry_run:
            try:
                p.set_values(new, str(REPO))
            except p.PostureError as error:
                raise Failure(str(error)) from None
        return {}
    if store not in p.REFERENCE_STORES:
        raise Failure(f"unknown secrets store {store!r}")
    directory = refs_dir(store)
    refs = read_refs(directory)
    alias, child = [], {}
    for x in suffixes:
        fn, sn = f"{f}_{x}", f"{s}_{x}"
        fref = refs.get(fn)
        if not fref or not fref.startswith(prefix):
            raise Refusal(FOUNDER_MISSING, f"{fn} has no {prefix} reference in {directory.name}/*.refs, so {sn} "
                          f"cannot take it: store it (python3 ops/secrets/posture.py hint {fn}), then resume")
        child[fn] = fref
        sref, senv = refs.get(sn), os.environ.get(sn)
        if sref == fref:
            print(f"{sn}: already names {fn}'s reference")
        elif sref or senv:
            print(f"{sn}: already held; the comparison decides")
        else:
            print(f"{sn} takes {fn} (the founder's reference)")
            alias.append((sn, fref))
        if sref or not senv:
            child[sn] = sref or fref
    if alias and not dry_run:
        append_refs(directory / "30-site-services.refs", site, founder, alias)
        print(f"wrote {len(alias)} reference(s) to {directory.name}/30-site-services.refs; a shell that "
              "loaded .envrc.local.d before this step does not hold them (direnv reload)")
    return child


def compare(site, founder, suffixes):
    """Step 3, run under with-secrets: names and same/differs only."""
    s, f = upper(site), upper(founder)
    prefix = os.environ.get("GOAT_SECRET_REF_PREFIX", "")
    missing, differs = [], []
    for x in suffixes:
        fv, sv = os.environ.get(f"{f}_{x}", ""), os.environ.get(f"{s}_{x}", "")
        if unresolved(fv, prefix):
            missing.append(f"{f}_{x}")
            print(f"{s}_{x}: {f}_{x} does not resolve")
        elif unresolved(sv, prefix):
            missing.append(f"{s}_{x}")
            print(f"{s}_{x}: does not resolve")
        elif hmac.compare_digest(hashlib.sha256(sv.encode()).digest(), hashlib.sha256(fv.encode()).digest()):
            print(f"{s}_{x}: same as {f}_{x}")
        else:
            differs.append(f"{s}_{x}")
            print(f"{s}_{x}: differs from {f}_{x}")
    if differs:
        raise Refusal(JOINER_DIFFERS, f"{', '.join(differs)} already hold values that are not the founder's; "
                      "they are not overwritten: decide which is right and remove the other")
    if missing:
        raise Refusal(FOUNDER_MISSING, f"{', '.join(missing)} did not resolve after the secrets injector ran: "
                      "sign in to the store's tool, then resume")
    print(f"every shared name of {site} resolves to {founder}'s value")


def main(argv):
    sys.stdout.reconfigure(line_buffering=True)
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("site")
    ap.add_argument("--founder")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--compare", action="store_true", help=argparse.SUPPRESS)
    args = ap.parse_args(argv)
    if not SITE_RE.fullmatch(args.site) or not args.founder or not SITE_RE.fullmatch(args.founder):
        raise Failure("usage: join-site-secrets.py <site> --founder <founder> [--dry-run]")
    if args.site == args.founder:
        raise Failure("a founder takes nothing from itself; its values come from standup")
    suffixes = shared_suffixes(args.site)
    if args.compare:
        compare(args.site, args.founder, suffixes)
        return 0
    store = posture().store(str(REPO))
    child = take(args.site, args.founder, suffixes, store, args.dry_run)
    gen = ["bash", str(HERE / "newsite-secrets.sh"), args.site] + (["--dry-run"] if args.dry_run else [])
    # The generator mints only what is absent. Each shared name is present to it, so it
    # never mints one for a joiner, also on a dry run, where step 1 wrote nothing.
    gen_env = dict(os.environ)
    for x in suffixes:
        gen_env.setdefault(f"{upper(args.site)}_{x}", "join-site-secrets:taken-from-the-founder")
    if subprocess.run(gen, env=gen_env).returncode != 0:
        raise Failure("newsite-secrets.sh did not mint the site's own values")
    if args.dry_run:
        print("dry run: nothing written; the comparison runs on a real run")
        return 0
    env = dict(os.environ)
    for name, ref in child.items():
        if unresolved(env.get(name, ""), env.get("GOAT_SECRET_REF_PREFIX", "")):
            env[name] = ref
    env.pop("GOAT_SECRETS_RESOLVED", None)
    rc = subprocess.run([str(HERE / "with-secrets"), sys.executable, str(Path(__file__).resolve()),
                         args.site, "--founder", args.founder, "--compare"], env=env).returncode
    return rc


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except Refusal as r:
        print(str(r), file=sys.stderr)
        sys.exit(1)
    except Failure as e:
        print(f"join-site-secrets: {e}", file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        print("join-site-secrets: a store or local I/O failure; no value printed", file=sys.stderr)
        sys.exit(1)
