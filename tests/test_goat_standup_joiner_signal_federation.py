#!/usr/bin/env python3
"""ops/scripts/goat-standup.sh plans a joiner's signal federation in phase 9 with the
rows `goatbuilder standup` plans (joinnode.rs signal_federation_rows; the standup
parity gate, ADR 1201): the gate first, the token and proxy on the joiner then the
founder, the federation routers, the joiner's relay tunnel and pin, the delivery
proof, all before the data router. A joiner that declares
signal_federation_absent_reason gets the gate and a SKIP line; a founder gets none.

The parity gate itself runs in each overlay's CI against its goatnets; this holds
the script's rows here, on a synthetic joiner. With GOATBUILDER_BIN set to a built
goatbuilder, the binary's phase-9 rows for the same tree are compared too."""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CONFIG = """schema: goatbuilder/v1
preset: full
repo:
  source: example-org/goatnet-from-scratch
  image_namespace: ghcr.io/example-org
components:
  zenoh-router:
    declared_in: declared/pins.yaml
environments:
  - name: zagros
    converger: push-ansible
    sites:
      - name: alpha
        role: founder
        classes:
          core: {}
      - name: beta
        role: joiner
        joins: alpha
        classes:
          core: {}
promotion:
  path:
    - name: zagros
      kind: rung
      environment: zagros
"""


def tree(beta_extra: str = "") -> Path:
    """A throwaway checkout: the scripts and goatbuilder files, a founder alpha and a joiner beta."""
    root = Path(tempfile.mkdtemp())
    shutil.copytree(REPO / "ops/scripts", root / "ops/scripts")
    shutil.copytree(REPO / "ops/goatbuilder", root / "ops/goatbuilder")
    (root / "ops/goatbuilder/goatbuilder.yaml").write_text(CONFIG)
    gv = root / "ops/ansible/group_vars"
    (gv / "alpha_hosts").mkdir(parents=True)
    (gv / "beta_hosts").mkdir(parents=True)
    (gv / "alpha_hosts/vars.yml").write_text("goatnet_founder: true\nparticipates_in_spock: true\n")
    (gv / "beta_hosts/vars.yml").write_text(
        "goatnet_founder: false\njoin_founder_site: alpha\nparticipates_in_spock: true\n"
        "site_dns_zone: beta.example\nmesh_ip_a: \"192.0.2.20\"\n" + beta_extra)
    inv = root / "ops/ansible/inventory"
    inv.mkdir(parents=True)
    (inv / "mgmt-hosts.ini").write_text(
        "[mgmt_hosts:children]\nalpha_hosts\nbeta_hosts\n"
        "[alpha_hosts]\nmgmt-alpha-01 ansible_host=192.0.2.10\n"
        "[beta_hosts]\nmgmt-beta-01 ansible_host=192.0.2.20\n")
    return root


def script_rows(root: Path, site: str):
    """(phase, kind, label, command) for every phase row of the script's --plan."""
    env = {"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp()}
    r = subprocess.run(["bash", str(root / "ops/scripts/goat-standup.sh"), site, "--plan"],
                       capture_output=True, text=True, env=env, timeout=180)
    assert r.returncode == 0, r.stdout + r.stderr
    rows, lines = [], r.stdout.splitlines()
    for i, line in enumerate(lines):
        if line.startswith("=== Phase ") and line.endswith(" ==="):
            head = line[len("=== Phase "):-len(" ===")]
            n, rest = head.split(" ", 1)
            kind, label = rest[1:].split("] ", 1)
            cmd = lines[i + 1].strip() if i + 1 < len(lines) else ""
            rows.append((int(n), kind, label, cmd))
    return rows


def want_rows(root: Path, pre: str, inv: str, skip: bool):
    """The rows joinnode.rs signal_federation_rows writes for beta joining alpha."""
    r = str(root)
    a = f"cd '{r}/ops/ansible' && "
    need = ("{ [ -f inventory/hosts-alpha.ini ] || { echo 'alpha relay inventory inventory/hosts-alpha.ini "
            "missing — generate it: inventory/generate-relays.sh alpha terraform-alpha'; exit 1; }; }")
    gate = (9, "GATE", "[signal federation] the repo declares it on both sides: beta_hosts and alpha_hosts crossed "
            "and complete, alpha's relay inventory pins each federation hostname to its own node, the names "
            "resolve to the relays, the shared token is in the environment",
            f"python3 '{r}/ops/scripts/signal-federation-check.py' beta --peer alpha --resolve --require-env")
    if skip:
        return [gate, (9, "SKIP", "[signal federation] declared off at beta (signal_federation_absent_reason in "
                       "group_vars/beta_hosts); the gate above proved no side expects it: alpha is off too, or "
                       "federates with another site",
                       "echo '>> signal federation: declared off for beta (signal_federation_absent_reason); nothing to converge'")]
    return [
        gate,
        (9, "A", "[signal federation] token + dual-path proxy on the joiner beta first (the token is enforced "
         "before any public path reaches it)", f"{a}{pre} {inv} signal-federation-transport.yml -e federation_site=beta"),
        (9, "A", "[signal federation] token + dual-path proxy on the founder alpha (the same change, not a "
         "follow-up: one direction alone leaves offers undeliverable)",
         f"{a}{pre} {inv} signal-federation-transport.yml -e federation_site=alpha"),
        (9, "A", "[signal federation] each node's federation router and its certificate (Traefik, DNS-01)",
         f"{a}{pre} {inv} traefik-acme.yml -e target=beta_hosts && {pre} {inv} traefik-acme.yml -e target=alpha_hosts"),
        (9, "A", "[signal federation] the joiner's tunnel to every alpha relay (site-rathole-client.yml, service "
         "rathole_client_service_name)",
         f"{a}{need} && {pre} -i inventory/hosts-alpha.ini {inv} site-rathole-client.yml -e rathole_client_target=beta_hosts"),
        (9, "A", "[signal federation] alpha's relays: the beta backend and its federation-hostname pin, "
         "re-rendered and post-checked (relay-ingress-add-site.yml)",
         f"{a}{need} && {pre} -i inventory/hosts-alpha.ini -i inventory/alpha-goat-relay-alias.ini {inv} "
         "relay-ingress-add-site.yml -e target=goat_relay_hosts -e founder_site=alpha -e join_site=beta"),
        (9, "GATE", "[signal federation] proven from both nodes: transport up both ways, an offer forwarded and "
         "delivered", f"{a}{pre} {inv} signal-federation-verify.yml -e federation_site=beta -e federation_peer_site=alpha"),
    ]


def pre_inv(root: Path, rows):
    """The script's ansible prefix and inventories, read from its data-router row."""
    router = next(c for n, k, l, c in rows if l == "zenoh router")
    body = router[len(f"cd '{root}/ops/ansible' && "):router.index(" zenoh-deploy.yml")]
    pre, inv = body.split(" -i inventory/mgmt-hosts.ini", 1)
    return pre, "-i inventory/mgmt-hosts.ini" + inv


class JoinerSignalFederation(unittest.TestCase):
    def check(self, skip: bool):
        root = tree("signal_federation_absent_reason: one node holds every peer\n" if skip else "")
        try:
            rows = script_rows(root, "beta")
            fed = [r for r in rows if r[2].startswith("[signal federation]")]
            pre, inv = pre_inv(root, rows)
            self.assertEqual(fed, want_rows(root, pre, inv, skip))
            at = [r[2] for r in rows]
            self.assertLess(at.index(fed[-1][2]), at.index("zenoh router"), "before the data router")
            bin_ = os.environ.get("GOATBUILDER_BIN")
            if bin_:
                out = subprocess.run([bin_, "-C", str(root), "-o", "json", "standup", "zagros", "--site", "beta", "--dry-run"],
                                     capture_output=True, text=True, timeout=180,
                                     env={"PATH": os.environ["PATH"], "HOME": tempfile.mkdtemp(), "GOAT_MESH_PROBE": "0"})
                self.assertEqual(out.returncode, 0, out.stderr)
                checks = json.loads(out.stdout)["checks"]
                heads = [c["detail"] for c in checks if c["name"] == "phase" and "[signal federation]" in c["detail"]]
                self.assertEqual(heads, [f"=== Phase {n} [{k}] {l} ===" for n, k, l, _ in fed])
        finally:
            shutil.rmtree(root)

    def test_a_joiner_plans_the_federation_rows_before_its_router(self):
        self.check(skip=False)

    def test_a_joiner_that_declares_it_off_gets_the_gate_and_a_skip(self):
        self.check(skip=True)

    def test_a_founder_plans_no_federation_row(self):
        root = tree()
        try:
            self.assertFalse([r for r in script_rows(root, "alpha") if "signal federation" in r[2]])
        finally:
            shutil.rmtree(root)


if __name__ == "__main__":
    unittest.main()
