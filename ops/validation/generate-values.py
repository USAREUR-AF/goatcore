#!/usr/bin/env python3
"""generate-values.py — derive a goatnet's acceptance-gate value set from its own config.

The phase-13 acceptance gate (`run-acceptance.py`) resolves every probe against a
flat `--values` map (site_dns_zone, mesh_ip_a, orin_a, the applies_when flags, …).
Nothing produced that map, so a fresh goatnet reached the finish gate and every row
came back `<UNRESOLVED>` — a gate that cannot render a verdict. This closes that gap:
it reads the goatnet's group_vars + inventory and writes
`ops/validation/values-<env>.yaml`, so the gate is resolvable by construction.

It derives, never invents: every value comes from the committed config
(`group_vars/<group>/vars.yml`, the presence of `pubsub-state.yml`, and the
inventory group's hosts). Values it cannot find are simply omitted — the gate then
reports those rows UNKNOWN with a named reason, which is honest, not a false pass.

Usage:  ops/validation/generate-values.py <env>            # writes ops/validation/values-<env>.yaml
        ops/validation/generate-values.py <env> --stdout   # print, write nothing
        ops/validation/generate-values.py --self-test
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
GV = REPO / "ops/ansible/group_vars"
INV = REPO / "ops/ansible/inventory/mgmt-hosts.ini"
# The mesh override inventory (loaded AFTER mgmt-hosts.ini by every play that
# reaches a goatnet over its mesh). mgmt-hosts.ini carries LAN or SSM addresses
# a laptop cannot reach, and the acceptance rows the daily review prescribes
# must be able to SSH the host without a hand --var. Addresses here win when present.
MESH_INV = REPO / "ops/ansible/inventory/mgmt-hosts-mesh.ini"

# group_vars leaf keys the gate's value set needs, and their default when absent.
SCALAR_KEYS = {
    "site_dns_zone": None,
    "public_mgmt_fqdn": None,      # falls back to site_dns_zone below
    "mesh_ip_a": None,
    "mesh_ip_b": None,
    "name_snapshot_enabled": "false",
    "onboard_runner_enabled": "false",
    "has_obs_host": "false",
    # ADR 1183 rescue classes — gate the break-glass rows. A site that declares
    # it has none reads N/A, which is the honest verdict; a site that declares
    # one and never converged it reads FAIL, which is the point.
    "has_breakglass_node": "false",
    "has_network_jump_node": "false",
    "intra_site_pair": "false",
    "rosenpass_enabled": "false",  # gates the PQ-negotiated acceptance row
    # Federation axis (ADR 1084). participates_in_spock = this node runs
    # inter-site Spock at all; goatnet_founder = true (the default when unset,
    # per node-addition.md §3) means this node IS the goatnet's origin, false
    # means it JOINS an existing one. Both are needed to derive `spock_joiner`
    # below, the predicate the inter-site federation rows gate on.
    "participates_in_spock": "false",
    "goatnet_founder": "true",
    # The assay's probe identity (goatnet-acceptance-suite.md §"The probe
    # identity"): when true, the goat CLI rows run as the non-person context
    # `assay-probe` on the site's first management machine.
    "assay_probe_enabled": "false",
}
# onboard_runner_enabled gates the onboard-runner rows; it was missing here, so
# `onboard-runner-reaches-mgmt-api` read N/A on every evidence run and the row
# added for the outage could never have fired.
BOOL_KEYS = ("name_snapshot_enabled", "has_obs_host", "intra_site_pair", "rosenpass_enabled",
             "onboard_runner_enabled", "participates_in_spock", "goatnet_founder",
             "has_breakglass_node", "has_network_jump_node", "luks_tang_unlock",
             "assay_probe_enabled")


def _extract(text, key):
    """Value of `key: <value>` from an ansible vars file, robust to jinja/comments.
    Returns the stripped string, or None if absent/empty."""
    m = re.search(rf'^{re.escape(key)}:[ \t]*(.*?)[ \t]*(?:#.*)?$', text, re.M)
    if not m:
        return None
    v = m.group(1).strip().strip('"').strip("'").strip()
    return v or None


def _bool(v):
    return str(v).strip().lower() in ("true", "yes", "1", "on")


def _yaml_list(text, key):
    """The `- item` entries of a simple `key:` block in an ansible vars file.
    Walks the block (tolerating comments/blank lines between entries) and stops
    at the next key at any indent. Returns [] if the key is absent. Used for
    `inter_site_spock_peers`, the joiner's federation peer list."""
    lines = text.splitlines()
    try:
        i = next(n for n, ln in enumerate(lines)
                 if re.match(rf'^{re.escape(key)}:[ \t]*(?:#.*)?$', ln))
    except StopIteration:
        return []
    out = []
    for line in lines[i + 1:]:
        st = line.strip()
        if not st or st.startswith("#"):
            continue
        if not st.startswith("-"):
            break
        v = st.lstrip("-").strip().strip('"').strip("'")
        if v:
            out.append(v)
    return out


def _router_cn(env, manifests_dir=None):
    """The router server-cert CN from the goatnet's own cert manifest.

    Per-goatnet by design and the source of truth for what the router presents
    on :7447 (site-a → site-a-zenoh-router). Returns
    None when the manifest or the router entry is absent, so the row reports
    UNKNOWN with a named reason rather than a false pass."""
    d = Path(manifests_dir) if manifests_dir else (REPO / "ops/enrollment/manifests")
    f = d / f"{env}-certs.yaml"
    if not f.is_file():
        return None
    block = re.search(r"^\s*-\s*name:\s*router\s*$(.*?)(?=^\s*-\s*name:|\Z)",
                      f.read_text(), re.M | re.S)
    if not block:
        return None
    cn = re.search(r"^\s*cn:\s*(\S+)", block.group(1), re.M)
    return cn.group(1).strip() if cn else None


def _group_vars_text(gv_dir):
    """Every vars file of the group, as one text, highest precedence first.

    Ansible loads every file in group_vars/<group>/, not just vars.yml, and a
    site may split its registry out (for example, isitup_services kept in isitup.yml).
    Reading vars.yml alone dropped those keys, so the rows that expand over
    them read UNKNOWN. Ansible loads the files in name order and the later one
    wins; the readers here take the first match, so the files go in reverse
    name order."""
    files = [f for f in Path(gv_dir).iterdir()
             if f.is_file() and f.suffix in (".yml", ".yaml")]
    return "\n".join(f.read_text() for f in sorted(files, reverse=True))


def _spock_node_name(site_id, intra_pair):
    """The Spock node name for a site — spock-mesh-peering/tasks/main.yml:105-106:
    <site_id underscored>, plus '_a' when the site is an intra-site A/B pair."""
    return site_id.replace("-", "_") + ("_a" if intra_pair else "")


def generate(env, gv_dir=None, inv_path=None):
    group = env.replace("-", "_") + "_hosts"
    gv_dir = Path(gv_dir) if gv_dir else GV / group
    inv_path = Path(inv_path) if inv_path else INV
    vars_file = gv_dir / "vars.yml"
    if not vars_file.is_file():
        raise SystemExit(f"generate-values: no group_vars for '{env}' at {vars_file}")
    text = _group_vars_text(gv_dir)

    values = {"site_id": env}
    # The goat CLI legs of the assay. The context that holds a fabric identity
    # for this goatnet is named after it by convention; the probe runs it on
    # this machine unless `--var goat_via=user@host` names one that holds it.
    # goatbuilder reads the deploy ledgers by the inventory group's stem.
    values["goat_context"] = env
    values["goat_via"] = "localhost"
    # The candidate-intake rows mint, vouch and withdraw as a dedicated
    # non-person sponsor identity capped at browse, never as an operator
    # (ADR 1207). Its context carries this name wherever the rows run goat.
    values["goat_sponsor_context"] = "acceptance"
    values["goatbuilder_env"] = group[: -len("_hosts")]
    for key, default in SCALAR_KEYS.items():
        v = _extract(text, key)
        if v is None:
            v = default
        if v is None:
            continue
        # public_mgmt_fqdn is sometimes "{{ site_dns_zone }}" — resolve that one ref.
        if "{{" in v:
            v = v.replace("{{ site_dns_zone }}", values.get("site_dns_zone", "")).strip()
            if "{{" in v:      # still templated → cannot resolve here, omit (row → UNKNOWN)
                continue
        if key in BOOL_KEYS:
            values[key] = _bool(v)
        else:
            values[key] = v
    values.setdefault("public_mgmt_fqdn", values.get("site_dns_zone"))

    # pubsub-state replication applies only if the goatnet declares it.
    values["pubsub_state_repl_enabled"] = (gv_dir / "pubsub-state.yml").is_file()

    # The router server-cert CN the fabric/router row asserts on :7447. It is
    # per-goatnet BY DESIGN (for example site-a-zenoh-router) and read
    # straight from the cert manifest — the source of truth for what the router
    # presents — NOT a fleet-wide literal. The row used to hardcode
    # `goat-zenoh-router`, which no goatnet's router server cert uses: that CN is
    # the ADR-1020 cross-router FEDERATION peer subject (config.json5.j2), not the
    # listen cert, so the literal graded every goatnet FAIL including the founder.
    # The router's REST port on the management host (roles/goat-zenoh publishes
    # it on 127.0.0.1). The rows that probe the router once ran `curl` inside a
    # container named `goat-zenoh`, which no goatnet runs and whose image has no
    # curl, behind a retired `zenoh_enabled` gate: they read N/A everywhere.
    port = _extract(text, "goat_zenoh_rest_port")
    values["zenoh_rest_port"] = port if port and port.isdigit() else "8000"
    rcn = _router_cn(env)
    if rcn:
        values["zenoh_router_cn"] = rcn

    # orin_a/b/c = an SSH target (user@addr) per host in the inventory group,
    # ordered by router_role (A,B,C). A single-host goatnet yields only A.
    hosts = _inventory_hosts(inv_path, group)
    if inv_path == INV and MESH_INV.is_file():
        hosts.update(_inventory_hosts(MESH_INV, group))
    # The obs host is its own inventory group (<site>_obs_hosts, ADR 0826) and
    # is never a member of <site>_hosts, so read it too: it is where orin_c
    # comes from, and every observability + state-of-record row targets it.
    # Without this the obs rows resolved to <UNRESOLVED> and the
    # whole class grades UNKNOWN.
    if group.endswith("_hosts"):
        hosts.update(_inventory_hosts(inv_path, group[: -len("_hosts")] + "_obs_hosts",
                                      default_role="C"))
    for role, target in hosts.items():
        values[f"orin_{role.lower()}"] = target

    # Prefer the MESH IP as the acceptance SSH target. The probes ssh to
    # {orin_a} directly from wherever the gate runs; a jump-fronted venue's LAN
    # ansible_host (behind a bastion) is unreachable from a mesh
    # vantage, and a pair whose mesh-inventory lines omit router_role collides
    # both hosts onto role A (orin_a resolved to core-b with the wrong user).
    # The site's mesh_ip_a/b ARE reachable from any mesh peer and are keyed to
    # router_role A/B by convention, so use them, taking the role from the base
    # inventory and the login user from the mesh inventory, keyed by host name.
    # No-op for a site whose inventory addr already IS its mesh IP.
    role_users = _role_users(inv_path, MESH_INV if inv_path == INV else None, group)
    for role, ip_key in (("A", "mesh_ip_a"), ("B", "mesh_ip_b")):
        ip = values.get(ip_key)
        if ip and role in role_users:
            values[f"orin_{role.lower()}"] = f"{role_users[role]}@{ip}"

    # The F-093 separate-mount rule, as the site declares it: a host whose NVMe
    # IS the rootfs declares `require_docker_data_separate_mount: false` (the
    # baseline's own Phase 0.5 gate reads the same key), and the data-root row
    # reads N/A there instead of failing a deviation the site recorded.
    sep = _extract(text, "require_docker_data_separate_mount")
    values["docker_data_separate_mount"] = "false" if sep and sep.lower() in ("false", "no") else "true"

    # all_orins: every management machine of the site (core and obs), for the
    # rows that hold on each of them (`hosts_from: all_orins`). The row that
    # used it named it as a literal host and could never resolve.
    values["all_orins"] = [values[k] for k in ("orin_a", "orin_b", "orin_c") if values.get(k)]
    # The goat CLI rows that read the fabric run as probe_context on probe_via.
    # A goatnet that declares the assay probe identity runs them as that
    # non-person context on its first management machine, where
    # ops/scripts/create-assay-probe-identity.sh installed it; otherwise they
    # fall back to goat_context/goat_via (run-acceptance.py), the operator's own.
    if values.get("assay_probe_enabled") is True and values.get("orin_a"):
        values["probe_context"] = "assay-probe"
        values["probe_via"] = values["orin_a"]

    # breakglass_host — the rescue node's own SSH target (ADR 1183, standup phase 14).
    # Its inventory is a per-site file, not the mgmt one, and its rows target
    # the node itself: "is the clock-recovery path armed" cannot be asked from
    # anywhere else. Unresolved leaves the row UNKNOWN rather than green.
    if group.endswith("_hosts"):
        bg_group = group[: -len("_hosts")] + "_breakglass_hosts"
        bg_inv = INV.parent / f"{values.get('site_id', '')}-breakglass-nodes.ini"
        bg = _inventory_hosts(bg_inv, bg_group)
        if not bg:
            bg = _inventory_hosts(inv_path, bg_group)
        if bg:
            values["breakglass_host"] = sorted(bg.values())[0]

    # luks_tang_unlock — does any host at this site unlock its data volume via
    # Tang? Tang bindings are declared per host (host_vars
    # luks_tang_servers), so read the host_vars of every host resolved above.
    # Gates the unattended-boot rows: a site with no Tang-unlocked volume reads
    # N/A rather than FAIL.
    hv_dir = INV.parent.parent / "host_vars"
    tang_hosts = [n for n in _inventory_host_names(inv_path, group)
                  + _inventory_host_names(inv_path, group[: -len("_hosts")] + "_obs_hosts")
                  if (hv_dir / f"{n}.yml").is_file()
                  and re.search(r'^luks_tang_servers:\s*\n\s+-', (hv_dir / f"{n}.yml").read_text(), re.M)]
    values["luks_tang_unlock"] = "true" if tang_hosts else "false"

    # list-valued vars the batch rows expand over.
    svcs = _isitup_services(text, values.get("site_id"), values.get("site_dns_zone"))
    if svcs:
        values["isitup_services"] = svcs
    containers = _isitup_containers(text)
    if containers:
        values["isitup_containers"] = containers
    fallback = _mesh_apex_fallback_ips(text)
    if fallback:
        values["mesh_apex_fallback_ips"] = fallback
    relays = _relay_public_ips(values.get("public_mgmt_fqdn"))
    if relays:
        values["relay_public_ips"] = relays

    # public login/consent surfaces the Safe Browsing gate row checks: the
    # enrollment apex plus every declared service reachable off-mesh
    # (access: public). A fresh one of these is what Google flags.
    pub = _public_auth_fqdns(text, values.get("site_id"),
                             values.get("site_dns_zone"), values.get("public_mgmt_fqdn"))
    if pub:
        values["public_auth_fqdns"] = pub

    # HOW A CURL ON THE HOST TRUSTS THIS GOATNET'S OWN EDGE. A site on its own
    # CA (file-delivered certs) carries its root at the portal role's bundle
    # path; without it the portal-health row read "self-signed certificate in
    # certificate chain" on a goatnet whose portal was answering.
    if (_extract(text, "dashboard_cert_delivery_mode") or "").strip().lower() == "file":
        values["curl_ca_opt"] = f"--cacert /etc/{values.get('site_id')}-ca/root-cert.pem"
    else:
        values["curl_ca_opt"] = ""

    # WHERE THIS GOATNET'S PROMETHEUS IS, whichever stack renders it.
    #
    # The observability rows used to key on `has_obs_host` and hardcode
    # `{{ orin_c }}` + localhost:9090. `has_obs_host` answers "does this SITE
    # have a separate obs node" — a topology fact — and was being read as "does
    # this site have monitoring". They are different questions, and the same
    # confusion is already written down in roles/deadman-watch/tasks/main.yml,
    # where testing `has_obs_host` for "is this host the obs host" got the guard
    # inverted in both directions.
    #
    # A goatnet can run Prometheus as an obs host or as a co-resident stack, and
    # both must be checked: an observability row that applies to one shape lets
    # a relay-certificate scrape job sit unconverged with the gate green.
    #
    # Deriving the location instead of the topology makes every one of them
    # checkable with no per-site opt-in:
    #   dedicated obs node  -> orin_c, obs-host stack, :9090, /opt/obs-host/…
    #   co-resident         -> orin_a, goat-obs stack, :9192, /opt/<site>-obs/…
    # A goatnet that runs no Prometheus at all yields no host, and the rows stay
    # correctly inapplicable rather than being switched off by a hand-set flag.
    ded = _bool(values.get("has_obs_host"))
    prom_host = values.get("orin_c") if ded else values.get("orin_a")
    if prom_host:
        values["obs_prometheus_host"] = prom_host
        values["obs_prometheus_url"] = "http://localhost:9090" if ded else "http://localhost:9192"
        values["obs_alertmanager_config"] = (
            "/opt/obs-host/alertmanager.yml" if ded
            else f"/opt/{values.get('site_id')}-obs/alertmanager/alertmanager.yml")

    # Declared probe targets, so the gate can ask whether they are actually
    # SCRAPED. A goatnet can declare these in its obs group_vars, not the site's,
    # so read both: reading only the site file leaves that declaration invisible
    # to every check.
    tls = _obs_relay_tls_targets(text, gv_dir, group)
    if tls:
        values["obs_relay_tls_targets"] = tls

    # ----------------------------------------------------------------------
    # FEDERATION (JOINER) axis — ADR 1084. The inter-site Spock acceptance
    # rows must fire ONLY on a node that JOINS an existing goatnet, never on
    # the founder (where seed-from-peer never ran) and never on a single-site
    # goatnet. run-acceptance's applies_when takes a single truthy var name,
    # so the predicate is derived here:
    #
    #   spock_joiner = participates_in_spock AND NOT goatnet_founder
    #
    # The founder ALSO carries participates_in_spock: true, so gating on that
    # alone would wrongly fire the join-proof rows on the founder. A single-
    # site goatnet has participates_in_spock: false and emits none of the
    # derived values below, so those rows stay correctly N/A.
    # ----------------------------------------------------------------------
    joiner = _bool(values.get("participates_in_spock")) and not _bool(
        values.get("goatnet_founder", "true"))
    values["spock_joiner"] = joiner
    if joiner:
        sid = values["site_id"]
        intra = _bool(values.get("intra_site_pair"))
        values["spock_local_node"] = _spock_node_name(sid, intra)
        peers = _yaml_list(text, "inter_site_spock_peers")
        if peers:
            values["spock_peers"] = peers
            peer = peers[0]
            # Peers are single-host per site (node-addition.md §4), so the
            # peer's node name follows the same rule with no A/B suffix.
            values["spock_peer_node"] = _spock_node_name(peer, False)
            # The founder's goat context: the cross-site fabric rows publish
            # on one side and listen on the other.
            values["goat_context_peer"] = peer
            # The subscription that pulls FROM the peer INTO this node — its
            # status is what the joiner queries locally (sub_<provider>_to_<local>).
            values["spock_inbound_sub"] = (
                f"sub_{values['spock_peer_node']}_to_{values['spock_local_node']}")
            # The peer's Postgres :5432 DSN host = the peer group's
            # spock_mesh_publisher_host (:= mesh_ip_a). F-396 defect #2 was this
            # silently falling back to the PUBLIC enrolment apex, which resolves
            # to the relays and does NOT listen on 5432 — sub_create timed out.
            # A reachability row needs the real peer host to catch that.
            peer_vars = Path(gv_dir).parent / (peer.replace("-", "_") + "_hosts") / "vars.yml"
            if peer_vars.is_file():
                ptext = peer_vars.read_text()
                phost = _extract(ptext, "spock_mesh_publisher_host")
                if not phost or "{{" in phost:      # publisher_host is usually "{{ mesh_ip_a }}"
                    phost = _extract(ptext, "mesh_ip_a")
                if phost and "{{" not in phost:
                    values["spock_peer_dsn_host"] = phost
        # ory_narrow federates the kratos + hydra DBs (ADR 1084 D2); the ory
        # rows gate on this list being present (a netbird-only joiner omits it).
        if _bool(_extract(text, "ory_spock_enabled")):
            values["spock_ory_dbs"] = ["kratos", "hydra"]
        # The netbird migration marker the guarded seed-from-peer play REMOVES
        # on success (<site>-seed-from-peer.yml). Its ABSENCE on a joiner is
        # the seed-completed signal; PASS 1 of bootstrap touches it on the
        # empty DB, so a present marker means the seed never ran.
        values["spock_seed_marker"] = "/opt/mgmt-host/management-a/marker/migrations.allow"

    # The identity row reads the NetBird management config the mgmt-host play
    # writes ({{ compose_dir }}/management-a/management.json).
    compose_dir = _extract(text, "compose_dir") or "/opt/mgmt-host"
    if "{{" not in compose_dir:
        values["netbird_mgmt_config"] = f"{compose_dir}/management-a/management.json"
    # The enrolment rows write (a throwaway peer and its key, both removed), so
    # they run only when asked: --var enroll_probe=1 --var enroll_peer_host=<docker
    # host>. The client image is the goatnet's own NetBird client version; the
    # admin token follows the <SITE>_NB_API_TOKEN convention.
    values["enroll_probe"] = "false"
    values["enroll_peer_host"] = ""
    values["nb_api_token_env"] = env.upper().replace("-", "_") + "_NB_API_TOKEN"
    # The portal rows' bearer: the site's operator key, under the name every
    # secret store exports it as (newsite-secrets.sh: <SITE>_PORTAL_OPERATOR_API_KEY).
    values["portal_operator_key_env"] = env.upper().replace("-", "_") + "_PORTAL_OPERATOR_API_KEY"
    nbv = _extract(text, "netbird_version")
    if nbv and "{{" not in nbv:
        values["enroll_client_image"] = f"netbirdio/netbird:{nbv}"

    # The enrollment entrypoint's TLS issuer (tls-identity row
    # enrollment-entrypoint-cert-identity). Every goatnet in this tree today
    # sets dashboard_cert_delivery_mode: acme, so Traefik presents a Let's
    # Encrypt leaf; derive from that flag rather than hardcoding the literal,
    # so an internal-CA site (dashboard_cert_delivery_mode: file/other) is not
    # graded against an issuer it cannot present. No site yet names its
    # internal CA's CN in committed config, so that case is left unresolved
    # (derive, never invent) until one does, via internal_ca_cn.
    cert_mode = _extract(text, "dashboard_cert_delivery_mode")
    # WHERE the probe reaches that entrypoint from here. An internet site (acme,
    # the same test goat-standup.sh uses to pick the profile) is fronted by its
    # relays, and they publish only :443: rathole forwards it to the host's
    # enrollment entrypoint (rathole_client_netbird_mgmt_local_addr), so the
    # host's :8443 is not public and probing it read UNKNOWN. A LAN site's
    # apex is the host itself, whose enrollment entrypoint is :8443.
    values["enrollment_public_port"] = "443" if cert_mode == "acme" else "8443"
    if cert_mode == "acme":
        values["entrypoint_cert_issuer"] = "Let's Encrypt"
    elif cert_mode:
        ca_cn = _extract(text, "internal_ca_cn")
        if ca_cn:
            values["entrypoint_cert_issuer"] = ca_cn

    return values


def _obs_relay_tls_targets(text, gv_dir, group):
    """`obs_relay_tls_targets` for this goatnet — the relay endpoints it declares
    for TLS-expiry probing. Read from the site's group_vars and from its obs
    group's, because the two stacks put it in different places."""
    out, seen = [], set()
    texts = [text]
    obs_vars = Path(gv_dir).parent / (group[: -len("_hosts")] + "_obs_hosts") / "vars.yml"
    if obs_vars.is_file():
        texts.append(obs_vars.read_text())
    for t in texts:
        lines = t.splitlines()
        try:
            i = next(n for n, l in enumerate(lines)
                     if l.startswith("obs_relay_tls_targets:"))
        except StopIteration:
            continue
        # Walk the block, not a single regex run: a comment or blank line BETWEEN
        # entries must not end the list, because a regex that stops at one reads
        # only part of the declared targets and reports no error, which is the same
        # silent-undercount shape this whole change exists to remove.
        for line in lines[i + 1:]:
            st = line.strip()
            if not st or st.startswith("#"):
                continue
            if not st.startswith("-"):
                break                      # next key at any indent ends the list
            v = st.lstrip("-").strip().strip('"').strip("'")
            if v and v not in seen:
                seen.add(v)
                out.append(v)
    return out


def _inventory_host_names(inv_path, group):
    """Inventory host names (first token of each line) in `group`."""
    if not group or not Path(inv_path).is_file():
        return []
    names, in_group = [], False
    for line in Path(inv_path).read_text().splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            in_group = (s == f"[{group}]")
            continue
        if in_group and s and not s.startswith("#"):
            names.append(s.split()[0])
    return names


def _inventory_hosts(inv_path, group, default_role="A"):
    """{role: 'user@addr'} for the inventory group — an SSH target the probes use
    directly (`ssh {orin_a} …`). Role from router_role= (default A, or C for an
    obs group); user from ansible_user= on the host line, else from the group's
    `[group:vars]` section (where obs groups set it), else goat."""
    if not Path(inv_path).is_file():
        return {}
    out, in_group, in_vars, group_user = {}, False, False, None
    rows = []
    for line in Path(inv_path).read_text().splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            in_group = (s == f"[{group}]")
            in_vars = (s == f"[{group}:vars]")
            continue
        if not s or s.startswith("#"):
            continue
        if in_vars and s.startswith("ansible_user="):
            group_user = s.split("=", 1)[1].strip()
        if in_group:
            rows.append(s)
    for s in rows:
        kv = dict(re.findall(r'(\w+)=(\S+)', s))
        addr = kv.get("ansible_host")
        role = kv.get("router_role", default_role)
        user = kv.get("ansible_user", group_user or "goat")
        if addr and not addr.startswith("TBD_"):
            out[role] = f"{user}@{addr}"
    return out


def _role_users(base_inv, mesh_inv, group):
    """{router_role: ssh_user} for the group, keyed by HOST NAME so a pair whose
    mesh-inventory lines omit router_role does not collide. Role comes from
    whichever inventory declares it (the base carries router_role even when its
    addr is still a TBD_ placeholder); the login user from ansible_user."""
    role_by_name, user_by_name = {}, {}
    for inv in (base_inv, mesh_inv):
        if not inv or not Path(inv).is_file():
            continue
        in_group = False
        for line in Path(inv).read_text().splitlines():
            s = line.strip()
            if s.startswith("[") and s.endswith("]"):
                in_group = (s == f"[{group}]")
                continue
            if not s or s.startswith("#") or not in_group:
                continue
            name = s.split()[0]
            kv = dict(re.findall(r'(\w+)=(\S+)', s))
            if "router_role" in kv:
                role_by_name[name] = kv["router_role"]
            if "ansible_user" in kv:
                user_by_name[name] = kv["ansible_user"]
    return {role: user_by_name.get(name, "goat")
            for name, role in role_by_name.items()}


def _isitup_services(text, site_id, zone):
    """The declared service FQDNs the mesh DNS should resolve — taken from each
    isitup_services entry's own `fqdn:` field, NOT synthesised as
    <id>.<site>.<zone>. Many services do not follow that shape (goat-get serves
    at get.<zone>, onboarding-guide at guide.<zone>), so synthesising the name
    made the resolve check false-fail on real, resolvable services. The `fqdn:`
    field is the authority; its {{ site_id }}/{{ site_dns_zone }} jinja is
    resolved from the values we already have. Entries without a `fqdn:` are
    skipped (nothing to resolve)."""
    fqdns, in_block = [], False
    entry = {}

    def emit():
        # An `type: external` entry runs off the goatnet's hosts (a cloud VM
        # whose own record names its own address), so it is not expected to
        # resolve to the management host.
        f = entry.get("fqdn")
        if not f or entry.get("type") == "external":
            return
        if site_id:
            f = f.replace("{{ site_id }}", site_id)
        if zone:
            f = f.replace("{{ site_dns_zone }}", zone)
        if "{{" not in f:                     # only emit fully-resolved names
            fqdns.append(f)

    for line in text.splitlines():
        if re.match(r'^isitup_services:', line):
            in_block = True
            continue
        if in_block:
            if re.match(r'^\S', line):        # next top-level key ends the block
                break
            if re.match(r'^\s+-\s', line):     # a new entry
                emit()
                entry = {}
            m = re.match(r'^\s+(?:-\s+)?(fqdn|type):\s*(.+?)\s*(?:#.*)?$', line)
            if m:
                entry[m.group(1)] = m.group(2).strip().strip('"').strip("'").strip()
    emit()
    return fqdns


def _public_auth_fqdns(text, site_id, zone, apex):
    """The public login/consent surfaces the Safe Browsing gate row checks: the
    enrollment apex (the dashboard/login origin) plus every isitup_services
    entry declared `access: public` — the off-mesh-reachable surfaces, which
    are exactly the ones a fresh zone gets flagged on. Grouped per entry so the
    fqdn is only emitted when THAT entry is public; entries with no fqdn or a
    still-templated fqdn are skipped."""
    out = []
    if apex:
        out.append(apex)
    cur_fqdn = cur_access = None
    in_block = False

    def flush():
        if cur_fqdn and cur_access == "public":
            f = cur_fqdn
            if site_id:
                f = f.replace("{{ site_id }}", site_id)
            if zone:
                f = f.replace("{{ site_dns_zone }}", zone)
            if "{{" not in f and f not in out:
                out.append(f)

    for line in text.splitlines():
        if re.match(r'^isitup_services:', line):
            in_block = True
            continue
        if not in_block:
            continue
        if re.match(r'^\S', line):            # top-level key ends the block
            flush()
            break
        if re.match(r'^\s+-\s+id:', line):    # new entry — flush the previous
            flush()
            cur_fqdn = cur_access = None
            continue
        m = re.match(r'^\s+fqdn:\s*(.+?)\s*(?:#.*)?$', line)
        if m:
            cur_fqdn = m.group(1).strip().strip('"').strip("'").strip()
            continue
        m = re.match(r'^\s+access:\s*(\S+)', line)
        if m:
            cur_access = m.group(1).strip().strip('"').strip("'")
    else:
        flush()                               # block ran to EOF
    return out


def _isitup_containers(text):
    """The container name each declared service is backed by — the `container:`
    field of every isitup_services entry that has one. This is what makes the
    'actually deployed' gate row EXACT: a declared service whose container is not
    running is a hard failure, so a goatnet can't pass the gate missing services.
    Entries with no container (pure page/URL checks) contribute none."""
    out, in_block = [], False
    for line in text.splitlines():
        if re.match(r'^isitup_services:', line):
            in_block = True
            continue
        if in_block:
            if re.match(r'^\S', line):
                break
            m = re.match(r'^\s+container:\s*(\S+)', line)
            if m:
                c = m.group(1).strip().strip('"').strip("'")
                # skip jinja-templated container names ({{ zenoh_stack }}-router,
                # etc.) — cannot resolve them here without ansible, and those
                # services carry their own dedicated gate rows. A literal name only.
                if c and "{{" not in c:
                    out.append(c)
    return out


def _relay_public_ips(apex):
    """The relay round-robin = the PUBLIC A records of the enrollment apex, as
    a peer that is not yet enrolled sees them. Asked of a public resolver: on a
    machine that is a mesh peer, the system resolver hands the name to the mesh
    DNS, whose answer is the enrolled peers' fallback list (mesh IP first, see
    `_mesh_apex_fallback_ips`), not the public relay tier. Mesh addresses
    (100.64.0.0/10) are never relays. Empty (rows stay UNKNOWN) if it does not
    resolve."""
    if not apex:
        return []
    import ipaddress
    import shutil
    import socket
    import subprocess
    found = []
    if shutil.which("dig"):
        try:
            out = subprocess.run(["dig", "+short", "+time=3", "+tries=1", "A", apex, "@1.1.1.1"],
                                 capture_output=True, text=True, timeout=10)
            found = [l.strip() for l in out.stdout.splitlines() if re.match(r"^\d+\.\d+\.\d+\.\d+$", l.strip())]
        except (OSError, subprocess.TimeoutExpired):
            found = []
    if not found:
        try:
            infos = socket.getaddrinfo(apex, 443, proto=socket.IPPROTO_TCP)
            found = [i[4][0] for i in infos if ":" not in i[4][0]]
        except OSError:
            return []
    cgnat = ipaddress.ip_network("100.64.0.0/10")
    return sorted({ip for ip in found if ipaddress.ip_address(ip) not in cgnat})


def _mesh_apex_fallback_ips(text):
    """The relay addresses the mesh DNS hands enrolled peers as their fallback
    for the enrollment apex: the `dns_extra_hosts` entry whose fqdn is the
    zone apex, minus its mesh address. They must be this goatnet's own relays,
    the same set its public apex answers with."""
    out, in_entry = [], False
    for line in text.splitlines():
        if re.match(r'^\s+-\s+fqdn:\s*"?\{\{\s*site_dns_zone\s*\}\}"?\s*$', line):
            in_entry = True
            continue
        if in_entry:
            if re.match(r'^\s+-\s+fqdn:', line) or re.match(r'^\S', line):
                break
            m = re.match(r'^\s+-\s+"?(\d+\.\d+\.\d+\.\d+)"?', line)
            if m:
                out.append(m.group(1))
    import ipaddress
    cgnat = ipaddress.ip_network("100.64.0.0/10")
    return sorted({ip for ip in out if ipaddress.ip_address(ip) not in cgnat})


def render(values):
    lines = ["# Generated by ops/validation/generate-values.py — derived from the",
             f"# goatnet's own config. Do not hand-edit; re-generate. env={values['site_id']}"]
    for k in sorted(values):
        v = values[k]
        if isinstance(v, list):
            lines.append(f"{k}:")
            lines.extend(f'  - "{it}"' for it in v)
        else:
            v = "true" if v is True else "false" if v is False else f'"{v}"'
            lines.append(f"{k}: {v}")
    return "\n".join(lines) + "\n"


def self_test():
    import tempfile
    d = Path(tempfile.mkdtemp())
    (d / "gv").mkdir(); (d / "gv" / "vars.yml").write_text(
        'site_dns_zone: netbird.demo.net\nmesh_ip_a: "100.127.0.9"\n'
        'name_snapshot_enabled: true\nhas_obs_host: false\nintra_site_pair: false\n'
        'dashboard_cert_delivery_mode: acme\n')
    inv = d / "inv.ini"
    inv.write_text("[demo_hosts]\nmgmt-demo-01 ansible_host=100.127.0.9 router_role=A\n")
    v = generate("demo", gv_dir=d / "gv", inv_path=inv)
    problems = []
    if v.get("site_dns_zone") != "netbird.demo.net": problems.append("site_dns_zone not derived")
    if v.get("mesh_ip_a") != "100.127.0.9": problems.append("mesh_ip_a not derived")
    if v.get("name_snapshot_enabled") is not True: problems.append("name_snapshot_enabled not bool-true")
    if v.get("has_obs_host") is not False: problems.append("has_obs_host not bool-false")
    if v.get("orin_a") != "goat@100.127.0.9": problems.append("orin_a not user@addr")
    if v.get("public_mgmt_fqdn") != "netbird.demo.net": problems.append("public_mgmt_fqdn fallback failed")
    if v.get("pubsub_state_repl_enabled") is not False: problems.append("pubsub flag not false")
    if v.get("nb_api_token_env") != "DEMO_NB_API_TOKEN": problems.append("nb_api_token_env not <SITE>_NB_API_TOKEN")
    if v.get("portal_operator_key_env") != "DEMO_PORTAL_OPERATOR_API_KEY":
        problems.append("portal_operator_key_env not <SITE>_PORTAL_OPERATOR_API_KEY "
                        f"(got {v.get('portal_operator_key_env')!r})")
    if v.get("entrypoint_cert_issuer") != "Let's Encrypt": problems.append("acme site: entrypoint_cert_issuer should be Let's Encrypt")
    if v.get("enrollment_public_port") != "443":
        problems.append("acme (internet, relay-fronted) site: the enrollment entrypoint is "
                        f"reached on the relays' :443 (got {v.get('enrollment_public_port')!r})")
    if problems:
        print("SELF-TEST FAILED:"); [print("  -", p) for p in problems]; return 2

    # --- a registry split out of vars.yml (e.g. isitup.yml) -------
    # Ansible reads every file in the group dir; so must the value set, or the
    # rows that expand over the registry read UNKNOWN. vars.yml sorts after
    # isitup.yml, so it wins a key both set.
    d6 = Path(tempfile.mkdtemp())
    (d6 / "split_hosts").mkdir(parents=True)
    (d6 / "split_hosts" / "vars.yml").write_text(
        'site_dns_zone: netbird.split.net\nmesh_ip_a: "100.127.0.24"\n'
        'has_obs_host: false\n')
    (d6 / "split_hosts" / "isitup.yml").write_text(
        '---\nhas_obs_host: true\nisitup_services:\n'
        '  - id: auth\n    fqdn: "auth.{{ site_id }}.{{ site_dns_zone }}"\n'
        '    container: kratos\n    access: public\n'
        '  - id: mgmt\n    fqdn: "mgmt.{{ site_id }}.{{ site_dns_zone }}"\n    access: api\n')
    inv6 = d6 / "inv.ini"
    inv6.write_text("[split_hosts]\nmgmt-split-01 ansible_host=100.127.0.24 router_role=A\n")
    v6 = generate("split", gv_dir=d6 / "split_hosts", inv_path=inv6)
    if v6.get("isitup_services") != ["auth.split.netbird.split.net", "mgmt.split.netbird.split.net"]:
        problems.append(f"split registry: isitup_services not read from isitup.yml "
                        f"(got {v6.get('isitup_services')})")
    if v6.get("isitup_containers") != ["kratos"]:
        problems.append(f"split registry: isitup_containers (got {v6.get('isitup_containers')})")
    if v6.get("public_auth_fqdns") != ["netbird.split.net", "auth.split.netbird.split.net"]:
        problems.append(f"split registry: public_auth_fqdns (got {v6.get('public_auth_fqdns')})")
    if v6.get("has_obs_host") is not False:
        problems.append("split registry: vars.yml must win a key isitup.yml also sets")

    if problems:
        print("SELF-TEST FAILED:"); [print("  -", p) for p in problems]; return 2

    # --- enrollment entrypoint issuer: internal-CA site (ADR-less today; a
    # site declares its own CA's CN once it exists, never invented) ----------
    d4 = Path(tempfile.mkdtemp())
    (d4 / "ca_hosts").mkdir(parents=True)
    (d4 / "ca_hosts" / "vars.yml").write_text(
        'site_dns_zone: netbird.ca-site.net\nmesh_ip_a: "100.127.0.22"\n'
        'dashboard_cert_delivery_mode: file\ninternal_ca_cn: ca-site-internal-ca\n')
    inv4 = d4 / "inv.ini"
    inv4.write_text("[ca_hosts]\nmgmt-ca-01 ansible_host=100.127.0.22 router_role=A\n")
    v4 = generate("ca-site", gv_dir=d4 / "ca_hosts", inv_path=inv4)
    if v.get("curl_ca_opt") != "":
        problems.append(f"acme site: curl_ca_opt should be empty (got {v.get('curl_ca_opt')!r})")
    if v4.get("curl_ca_opt") != "--cacert /etc/ca-site-ca/root-cert.pem":
        problems.append(f"internal-CA site: curl_ca_opt should name the root on the host (got {v4.get('curl_ca_opt')!r})")
    if v4.get("entrypoint_cert_issuer") != "ca-site-internal-ca":
        problems.append(f"internal-CA site: entrypoint_cert_issuer should be the declared CN "
                        f"(got {v4.get('entrypoint_cert_issuer')!r})")

    # A non-acme site that names no internal_ca_cn yet must NOT get a value —
    # the row should read UNKNOWN, not FAIL against an invented issuer.
    d5 = Path(tempfile.mkdtemp())
    (d5 / "nocn_hosts").mkdir(parents=True)
    (d5 / "nocn_hosts" / "vars.yml").write_text(
        'site_dns_zone: netbird.nocn.net\nmesh_ip_a: "100.127.0.23"\n'
        'dashboard_cert_delivery_mode: file\n')
    inv5 = d5 / "inv.ini"
    inv5.write_text("[nocn_hosts]\nmgmt-nocn-01 ansible_host=100.127.0.23 router_role=A\n")
    v5 = generate("nocn", gv_dir=d5 / "nocn_hosts", inv_path=inv5)
    if "entrypoint_cert_issuer" in v5:
        problems.append("non-acme site with no internal_ca_cn: must not invent entrypoint_cert_issuer")
    if v5.get("enrollment_public_port") != "8443":
        problems.append("lan site: the enrollment entrypoint is the host's own :8443 "
                        f"(got {v5.get('enrollment_public_port')!r})")

    if problems:
        print("SELF-TEST FAILED:"); [print("  -", p) for p in problems]; return 2
    # --- observability location, both stacks -----------------------------
    # These four assertions are the point of the change: a goatnet is
    # checked because it RUNS a Prometheus, not because a hand-set topology flag
    # says it has a separate box for one.
    if v.get("obs_prometheus_host") != "goat@100.127.0.9":
        problems.append("co-resident obs: prometheus host should be orin_a")
    if v.get("obs_prometheus_url") != "http://localhost:9192":
        problems.append("co-resident obs: goat-obs serves :9192, not :9090")
    if v.get("obs_alertmanager_config") != "/opt/demo-obs/alertmanager/alertmanager.yml":
        problems.append("co-resident obs: alertmanager path should be per-site")

    # Dedicated obs node: same three values, resolved off the obs group.
    d2 = Path(tempfile.mkdtemp())
    (d2 / "ded_hosts").mkdir(parents=True)
    (d2 / "ded_hosts" / "vars.yml").write_text(
        'site_dns_zone: netbird.ded.net\nmesh_ip_a: "100.127.0.18"\n'
        'has_obs_host: true\nintra_site_pair: false\n')
    # …and its relay TLS targets declared in the OBS group, not the site group —
    # the placement that hides a declaration from every check.
    # The comment BETWEEN entries is deliberate: a regex-run reader stops there
    # and reports part of the targets with no error.
    (d2 / "ded_obs_hosts").mkdir(parents=True)
    (d2 / "ded_obs_hosts" / "vars.yml").write_text(
        'obs_relay_tls_targets:\n'
        '  - "relay-a.ded.net:33073"\n'
        '  # a sibling goatnet with no obs host of its own, watched here\n'
        '  - "relay-b.ded.net:33073"\n'
        'obs_blackbox_enabled: true\n')
    inv2 = d2 / "inv.ini"
    inv2.write_text("[ded_hosts]\nmgmt-ded-01 ansible_host=100.127.0.18 router_role=A\n"
                    "[ded_obs_hosts]\nmgmt-ded-obs ansible_host=100.127.0.20 router_role=C\n")
    v2 = generate("ded", gv_dir=d2 / "ded_hosts", inv_path=inv2)
    if v2.get("obs_prometheus_url") != "http://localhost:9090":
        problems.append("dedicated obs: obs-host serves :9090")
    if v2.get("obs_alertmanager_config") != "/opt/obs-host/alertmanager.yml":
        problems.append("dedicated obs: alertmanager path is the role's")
    if not (v2.get("obs_prometheus_host") or "").endswith("100.127.0.20"):
        problems.append("dedicated obs: prometheus host should be the obs group's node")
    if v2.get("obs_relay_tls_targets") != ["relay-a.ded.net:33073", "relay-b.ded.net:33073"]:
        problems.append("declared relay TLS targets: obs-group placement, comment mid-list "
                        f"(got {v2.get('obs_relay_tls_targets')})")

    if problems:
        print("self-test FAILED:")
        for pr in problems:
            print("  -", pr)
        return 1

    # --- federation joiner axis (ADR 1084) ---------------------------------
    # A joiner (participates_in_spock + goatnet_founder:false) with a peer group.
    d3 = Path(tempfile.mkdtemp())
    (d3 / "join_hosts").mkdir(parents=True)
    (d3 / "join_hosts" / "vars.yml").write_text(
        'site_dns_zone: netbird.join.net\nmesh_ip_a: "100.127.0.63"\n'
        'participates_in_spock: true\ngoatnet_founder: false\nintra_site_pair: false\n'
        'ory_spock_enabled: true\n'
        'inter_site_spock_topology: explicit\ninter_site_spock_peers:\n  - orig\n')
    # the founder ("orig") the joiner peers with — its mesh_ip_a is the DSN host.
    (d3 / "orig_hosts").mkdir(parents=True)
    (d3 / "orig_hosts" / "vars.yml").write_text(
        'site_dns_zone: netbird.join.net\nmesh_ip_a: "100.127.0.62"\n'
        'participates_in_spock: true\ngoatnet_founder: true\n'
        'spock_mesh_publisher_host: "{{ mesh_ip_a }}"\n')
    inv3 = d3 / "inv.ini"
    inv3.write_text("[join_hosts]\nmgmt-join-01 ansible_host=100.127.0.63 router_role=A\n")
    vj = generate("join", gv_dir=d3 / "join_hosts", inv_path=inv3)
    if vj.get("spock_joiner") is not True:
        problems.append("joiner: spock_joiner should be true")
    if vj.get("spock_local_node") != "join":
        problems.append(f"joiner: spock_local_node (got {vj.get('spock_local_node')})")
    if vj.get("spock_peer_node") != "orig":
        problems.append(f"joiner: spock_peer_node (got {vj.get('spock_peer_node')})")
    if vj.get("spock_inbound_sub") != "sub_orig_to_join":
        problems.append(f"joiner: spock_inbound_sub (got {vj.get('spock_inbound_sub')})")
    if vj.get("spock_peer_dsn_host") != "100.127.0.62":
        problems.append(f"joiner: peer DSN host should be the founder's mesh_ip_a "
                        f"(got {vj.get('spock_peer_dsn_host')})")
    if vj.get("spock_ory_dbs") != ["kratos", "hydra"]:
        problems.append(f"joiner: spock_ory_dbs (got {vj.get('spock_ory_dbs')})")
    if not (vj.get("spock_seed_marker") or "").endswith("migrations.allow"):
        problems.append("joiner: spock_seed_marker not derived")

    # A single-site founder emits NONE of the joiner values (rows stay N/A).
    vf = generate("orig", gv_dir=d3 / "orig_hosts", inv_path=inv3)
    if vf.get("spock_joiner") is not False:
        problems.append("founder: spock_joiner should be false")
    if "spock_inbound_sub" in vf or "spock_peer_dsn_host" in vf:
        problems.append("founder: must not emit joiner-only values")

    if problems:
        print("self-test FAILED:")
        for pr in problems:
            print("  -", pr)
        return 1
    print("self-test OK — values derived from config (scalars, bool flags, inventory host, "
          "fallback, obs location for both stacks, declared relay TLS targets, "
          "federation joiner axis).")
    return 0


def main():
    if "--self-test" in sys.argv:
        sys.exit(self_test())
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        raise SystemExit("usage: generate-values.py <env> [--stdout]")
    env = args[0]
    values = generate(env)
    out = render(values)
    if "--stdout" in sys.argv:
        sys.stdout.write(out)
    else:
        dest = HERE / f"values-{env}.yaml"
        dest.write_text(out)
        print(f"wrote {dest.relative_to(REPO)}  ({len(values)} values)")


if __name__ == "__main__":
    main()
