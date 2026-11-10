#!/usr/bin/env python3
"""Phase 1 of a peering standup: refuse before anything is measured or deployed.

WHAT THIS CHECKS, AND WHAT IT DELIBERATELY DOES NOT. `goat federate init` already
measures the running system and refuses, by name, for anything it cannot derive
(exit 23). Duplicating those probes here would be two homes for one fact and they
would drift. This checks only what init cannot: the inputs that must exist BEFORE
init is worth running, and the deployment surface that phases 8-10 will need long
after init has passed.

Fail-closed: every missing thing is named with the path it was looked for at.
Nothing is defaulted, nothing is created.

Exit 0 ready, 1 not ready.
"""
import argparse, os, re, sys
from pathlib import Path

OK, BAD = "[✓]", "[✗]"
problems = []


def ok(label, detail=""):
    print(f"{OK} {label}" + (f" — {detail}" if detail else ""))


def bad(label, detail, fix=""):
    print(f"{BAD} {label} — {detail}")
    if fix:
        for line in fix.strip().splitlines():
            print(f"     {line}")
    problems.append(label)


def inventory_groups(text):
    """Every group of an ini inventory and its host names, `:children` sections apart."""
    groups, children, cur, kids = {}, {}, None, False
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith(("#", ";")):
            continue
        if line.startswith("["):
            name = line[1:-1].strip()
            kids = name.endswith(":children")
            cur = name[: -len(":children")] if kids else name
            (children if kids else groups).setdefault(cur, [])
            continue
        if cur is not None:
            (children if kids else groups)[cur].append(line.split()[0])
    return groups, children


def shared_inventory_names(text, mine):
    """Host names in one of `mine` (this peering's edge and core groups) that also sit in
    another peering group. Ansible merges the group_vars of every group an inventory host
    is in, the group sorting last winning a conflict, so such a host's run reads the other
    peering's interface, ports and certificate paths. Returns [(host, [groups])]."""
    groups, children = inventory_groups(text)
    peering = set(children.get("peering_edges", [])) | set(children.get("peering_cores", []))
    found = []
    for g in mine:
        for h in groups.get(g, []):
            # The same peering's own edge and core groups naming one host is one
            # peering's variables, not two: not a conflict.
            others = sorted(o for o in peering if o not in mine and h in groups.get(o, []))
            if others and (h, [g] + others) not in found:
                found.append((h, [g] + others))
    return found


def check_local_docker():
    """Refuse an unverified phase-8 dependency before any deployment."""
    import shutil
    import subprocess

    label = "docker compose plugin (phase 12)"
    if not shutil.which("docker"):
        bad(label, "no docker on this host")
        return
    try:
        result = subprocess.run(["docker", "compose", "version"],
                                capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.SubprocessError) as exc:
        bad(label, str(exc))
        return
    if result.returncode != 0:
        bad(label, "docker has no compose plugin",
            "The edge role deploys with `docker compose`. Install\n"
            "docker-compose-plugin, or drop the plugin binary at\n"
            "/usr/libexec/docker/cli-plugins/docker-compose.")
    elif not result.stdout.strip():
        bad(label, "docker compose version returned no version")
    else:
        ok(label, result.stdout.strip().splitlines()[0])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("peering")
    ap.add_argument("--work", required=True)
    # Under `sudo -E` the invoking user's HOME survives, so Path.home() is
    # /home/<user> while the profile the run will actually read is root's. That
    # mismatch made the first driven run report three missing inputs that were
    # all present. Follow the effective user instead.
    ap.add_argument("--profile-dir", default=os.environ.get("GOAT_PROFILE_DIR", ""))
    ap.add_argument("--context", default=os.environ.get("GOAT_CONTEXT", "default"))
    ap.add_argument("--edge-group", default="")
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[3]))
    a = ap.parse_args()

    profile_dir = a.profile_dir or ("/root/.goat" if os.geteuid() == 0
                                    else str(Path.home() / ".goat"))
    print(f"preflight — peering {a.peering}")
    ctx = Path(profile_dir) / "contexts" / a.context

    # 1 — the declaration. init reads it and refuses without it, but it refuses
    #     AFTER printing a template, which reads like an error rather than a
    #     first step. Saying it here, first, is the difference.
    decl = ctx / "federation.toml"
    if not decl.is_file():
        bad("declaration", f"no federation.toml at {decl}",
            "It holds only what no probe can produce: the organisation's name, the term,\n"
            "the cut contact, and the link parameters two parties agree rather than one\n"
            "observes. `goat federate init` prints a template when it is absent.")
    else:
        txt = decl.read_text()
        m = re.search(r'^\s*sovereign_id\s*=\s*"([^"]+)"', txt, re.M)
        if not m:
            bad("declaration", f"{decl} has no sovereign_id")
        else:
            sid = m.group(1)
            if not (2 <= len(sid) <= 16 and re.fullmatch(r"[a-z0-9-]+", sid)):
                bad("sovereign_id", f"`{sid}` is not 2-16 chars of lowercase a-z0-9-")
            else:
                ok("declaration", f"sovereign_id `{sid}`")
                # ADVISORY, not a refusal: the shape is legal but the VALUE may
                # be an environment name, which the spec warns against because a
                # sandbox and a production fabric share one sovereign id. We
                # cannot know the organisation's real name, so we cannot decide
                # this for the operator -- only make sure it was a decision.
                for env in ("dev", "prod", "test", "sandbox", "staging", "lab"):
                    if sid == env or sid.endswith("-" + env):
                        print(f"     note: `{sid}` looks like an ENVIRONMENT name. A sovereign id is the")
                        print("     organisation, and a sandbox and a production fabric share one.")
                        break
        # A capability typed into the declaration is a claim; init refuses to
        # read one, but the operator finds out only after a full run.
        for measured in ("capabilities.transports", "capabilities.fabric_version",
                         "capabilities.fabric_fips_image", "capabilities.pqc_capable",
                         "transport.public_key"):
            leaf = measured.split(".")[-1]
            if re.search(rf'^\s*{re.escape(leaf)}\s*=', txt, re.M) and leaf != "public_key":
                bad("declaration", f"`{leaf}` is MEASURED and must not be declared",
                    "A capability typed by an operator is a claim, and a wrong claim stays\n"
                    "wrong until the link fails in front of a partner.")

    # 2 — profile identity. init derives trust_domain and ca_fingerprint from
    #     these two files; without them phase 1 cannot start.
    cert, roots = ctx / "mtls.cert.pem", ctx / "ca-roots.pem"
    if not cert.is_file():
        bad("profile certificate", f"none at {cert}", "Run `goat profile init <bundle.cbor>` first.")
    elif b"-----BEGIN CERTIFICATE-----" not in cert.read_bytes():
        bad("profile certificate", f"{cert} is not a PEM certificate")
    else:
        ok("profile certificate", str(cert))
    if not roots.is_file():
        bad("CA root bundle", f"none at {roots}")
    else:
        ok("CA root bundle", str(roots))

    # 3 — our signing key. Without it init prints the bytes to sign and stops,
    #     which is correct behaviour and a confusing way to discover the gap.
    key = os.environ.get("FED_SIGNING_KEY", "")
    if not key:
        bad("signing key", "FED_SIGNING_KEY is unset",
            "Our Ed25519 signing key: 64 hex characters (the RFC 8032 seed) or 44 base64.\n"
            "Without it `init` prints the exact bytes to sign and stops — it will not invent one.")
    elif Path(key).is_file():
        raw = Path(key).read_text().strip()
        if not (re.fullmatch(r"[0-9a-fA-F]{64}", raw) or len(raw) == 44):
            bad("signing key", f"{key} is neither 64 hex characters nor 44 base64")
        else:
            ok("signing key", key)
    else:
        bad("signing key", f"FED_SIGNING_KEY points at {key}, which does not exist")

    # 3b — the peering directory. Phase 5 exchanges packages and CA certificates
    #     through it, so a run without one would stop at phase 5 with nothing to
    #     exchange. FED_NO_DIRECTORY=1 is the one exception: a counterparty that
    #     can reach no directory, where phase 5 names the files to hand over.
    if os.environ.get("FED_NO_DIRECTORY", "") == "1":
        ok("peering directory", "FED_NO_DIRECTORY=1: phase 5 names the files to hand over directly")
    else:
        url = os.environ.get("FED_DIRECTORY_URL", "")
        their = os.environ.get("FED_COUNTERPARTY_ID", "")
        plain = re.compile(r"^[A-Za-z0-9._:/?=&%~+@-]*$")
        values = [url, their, os.environ.get("FED_PACKAGE_URL", ""), os.environ.get("FED_CA_CERT", "")]
        if not url:
            bad("peering directory", "FED_DIRECTORY_URL is unset",
                "Phase 5 exchanges packages and CA certificates through the peering directory\n"
                "(saltlick): set FED_DIRECTORY_URL=https://<directory> and FED_COUNTERPARTY_ID.\n"
                "FED_NO_DIRECTORY=1 only for a counterparty that can reach no directory.")
        elif not url.startswith("https://"):
            bad("peering directory", f"FED_DIRECTORY_URL is {url}, not an https:// address")
        elif not their:
            bad("peering directory", "FED_COUNTERPARTY_ID is unset",
                "The counterparty's sovereign_id (ISO 3166-1 alpha-3), to look them up.")
        elif not all(plain.match(v) for v in values):
            bad("peering directory", "FED_DIRECTORY_URL, FED_PACKAGE_URL, FED_COUNTERPARTY_ID or "
                "FED_CA_CERT holds characters outside a plain URL")
        else:
            ok("peering directory", f"{url}, counterparty {their}")

    # 4 — somewhere to work.
    work = Path(a.work)
    try:
        work.mkdir(parents=True, exist_ok=True)
        (work / ".w").write_text("x"); (work / ".w").unlink()
        ok("working directory", str(work))
    except OSError as e:
        bad("working directory", f"{work} is not writable: {e}")

    # 5 — the deployment surface phases 11-13 need. Checked NOW because it is
    #     free to check and expensive to discover at phase 11, after an agreement
    #     has been composed and certificates minted.
    group = a.edge_group or f"{a.peering}_edge"
    inv = Path(a.repo) / "ops/ansible/inventory/peering-edges.ini"
    if not inv.is_file():
        bad("edge inventory", f"no {inv}")
    elif f"[{group}]" not in inv.read_text():
        bad("edge inventory", f"{inv} has no [{group}] group",
            f"Phases 2 and 11-13 deploy with --limit {group}; a missing group deploys to nothing\n"
            f"and reports success.")
    else:
        # A group that EXISTS and is empty is the worse case: `--limit` matches
        # it, ansible runs against nothing, and the play reports success. The
        # committed inventory ships this group with its host commented out,
        # which is correct for the repository and fatal for a run.
        want, hosts = False, []
        for line in inv.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("["):
                want = line[1:-1].strip() == group
                continue
            if want:
                hosts.append(line.split()[0])
        if hosts:
            ok("edge inventory", f"[{group}] -> {', '.join(hosts)}")
            # Every peering play runs `hosts: peering_edges`. A group that is not a
            # child of it matches `--limit` and none of the plays' hosts, so ansible
            # prints "no hosts matched", exits 0, and every play "passes".
            children, inside = [], False
            for line in inv.read_text().splitlines():
                line = line.strip()
                if line.startswith("["):
                    inside = line == "[peering_edges:children]"
                elif inside and line and not line.startswith("#"):
                    children.append(line.split()[0])
            if group in children:
                ok("edge inventory", f"[{group}] is a child of [peering_edges]")
            else:
                bad("edge inventory", f"[{group}] is not listed under [peering_edges:children] in {inv}",
                    "Every peering play targets peering_edges, so `--limit` on this group\n"
                    "runs nothing and reports success.")
        else:
            bad("edge inventory", f"[{group}] exists but lists no hosts",
                "`--limit` on an empty group runs against nothing and reports SUCCESS,\n"
                "so phase 12 would 'pass' having deployed no edge at all.")

    # 5b — where the outbound republisher runs. When the declaration offers a
    #      release set, phase 12 deploys the republisher on our core host with
    #      --limit <peering>_core, under [peering_cores:children]. A missing or empty
    #      group would deploy nothing and report success, and nothing would ever
    #      appear under our wire name.
    offers = False
    if decl.is_file():
        m = re.search(r'^\s*release\s*=\s*\[([^\]]*)\]', decl.read_text(), re.M)
        offers = bool(m and m.group(1).strip().strip(","))
    core = f"{a.peering}_core"
    if offers and inv.is_file():
        want, hosts, children, inside = False, [], [], False
        for line in inv.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("["):
                want = line[1:-1].strip() == core
                inside = line == "[peering_cores:children]"
                continue
            if want:
                hosts.append(line.split()[0])
            if inside:
                children.append(line.split()[0])
        if not hosts:
            bad("republisher inventory", f"{inv} has no [{core}] group with a host in it",
                f"The declaration offers a release set, so phase 12 deploys the outbound\n"
                f"republisher on our core host with --limit {core}. Name the core host the\n"
                f"edge dials there; an empty or missing group deploys nothing and reports success.")
        elif core not in children:
            bad("republisher inventory", f"[{core}] is not listed under [peering_cores:children] in {inv}",
                "peering-republisher.yml targets peering_cores, so `--limit` on this group\n"
                "runs nothing and reports success.")
        else:
            ok("republisher inventory", f"[{core}] -> {', '.join(hosts)}")

    # 5c — one inventory name per peering. Two peerings on one machine are two inventory
    #      names for it (same ansible_host): one name in two peering groups merges both
    #      groups' variables, and a run for one peering reads the other's values.
    if inv.is_file():
        shared = shared_inventory_names(inv.read_text(), [group, f"{a.peering}_core"])
        for h, gs in shared:
            bad("one inventory name per peering", f"{h} is in {', '.join(gs)}",
                "Ansible merges the group_vars of every group a host is in and the group that\n"
                "sorts last wins, so this peering's run would read another peering's interface,\n"
                f"ports and certificate paths, and re-running either redeploys the other's values.\n"
                f"Give this peering its own inventory name for the machine, e.g. {h}-{a.peering},\n"
                "with the same ansible_host.")
        if not shared:
            ok("one inventory name per peering", "no host of this peering is in another peering's group")

    # C1 — declared first. goatbuilder refuses an undeclared peering, and the script
    #      holds the same rule here so neither avenue runs one by accident.
    cfg = Path(a.repo) / "ops/goatbuilder/goatbuilder.yaml"
    if cfg.is_file():
        names = re.findall(r'^\s*-?\s*with:\s*["\']?([^"\'\s#]+)', cfg.read_text(), re.M)
        if a.peering in names:
            ok("declared", f"`{a.peering}` is in {cfg} peerings:")
        else:
            bad("declared", f"`{a.peering}` is not in {cfg} peerings: (declared: {', '.join(names) or 'none'})",
                "Declare the peering first (C1). A peering nobody declared is one nobody reviewed.")

    # 6 — what phases 8, 9 and 12 need. Every one of these was discovered in the
    #     middle of the first driven run, AFTER an agreement had been composed
    #     and certificates minted, which is the expensive place to find them.
    ca_cert, ca_key = os.environ.get("FED_CA_CERT", ""), os.environ.get("FED_CA_KEY", "")
    if not ca_cert or not ca_key:
        bad("signing CA (phase 8)", "FED_CA_CERT / FED_CA_KEY unset",
            "Phase 8 mints this side's peering leaves and needs OUR OWN CA. Never the\n"
            "counterparty's: a foreign root near a core router IS an authorization grant,\n"
            "because the access layer matches a name and cannot see the issuer.")
    elif not Path(ca_cert).is_file() or not Path(ca_key).is_file():
        bad("signing CA (phase 8)", f"{ca_cert} / {ca_key} — one of these does not exist")
    else:
        ok("signing CA (phase 8)", ca_cert)

    # The core certificate must carry a SAN for every address the edge will dial.
    # Without it, name verification fails and this software logs nothing at INFO
    # when it does — so cross-check the two here rather than after a deploy.
    endpoints = []
    dep = Path(a.repo) / "ops/ansible/group_vars" / group / "deployment.yml"
    if dep.is_file():
        for line in dep.read_text().splitlines():
            m = re.search(r'"(?:tls|tcp)/([^:"/#]+):', line)
            if m:
                endpoints.append(m.group(1))
    san = os.environ.get("FED_CORE_SAN", "")
    if endpoints and not san:
        bad("core certificate SANs (phase 8)",
            f"FED_CORE_SAN is unset but the edge will dial {', '.join(endpoints)}",
            "A core certificate with no SAN for the dialled address fails name\n"
            "verification, silently.")
    elif endpoints:
        missing = [e for e in endpoints if e not in san.split(",")]
        if missing:
            bad("core certificate SANs (phase 8)",
                f"FED_CORE_SAN does not cover: {', '.join(missing)}",
                "The edge dials those; the core certificate must name them.")
        else:
            ok("core certificate SANs (phase 8)", san)

    # Phase 12 deploys with `docker compose`, and the module's error when the
    # plugin is absent arrives only after the config has been written.
    if os.environ.get("FED_SKIP_LOCAL_DOCKER_CHECK", "") != "1":
        check_local_docker()

    print()
    if problems:
        print(f"NOT READY — {len(problems)} thing(s) above. Nothing was measured, signed or deployed.")
        return 1
    print("READY — phase 1 can measure this host.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
