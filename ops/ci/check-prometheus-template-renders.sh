#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# check-prometheus-template-renders.sh — render the goat-obs Prometheus config
# for every goatnet that runs the stack, and assert each result is valid YAML
# with the jobs we expect.
#
# WHY THIS EXISTS: `ops/compose/goat-obs/prometheus/prometheus.yml` is an
# ANSIBLE TEMPLATE, not a Prometheus file. promtool cannot parse it (the Jinja
# is a syntax error to a YAML reader), so the `promtool check + unit tests` CI
# job deliberately checks only `rules/*.yml` and `tests/*.yml`. The scrape
# config itself — the part that decides what is watched at all — had no
# automated check of any kind. A broken `{% for %}` there does not fail CI; it
# fails at deploy, on the box, after the config is already in place, and
# Prometheus then refuses to start or silently drops a job.
#
# A hand-kept blackbox target list drifts: a path that is wrong (mgmt probed at
# `/` when its registry row declares `/api/users`) stays permanently red, and
# nothing else compares the probe to the declaration.
# Targets are now RENDERED from `isitup_services`, which removes that class of
# drift — and this guard is what keeps the rendering itself honest.
#
# Deliberately NOT a promtool check: promtool is a pinned download in the
# goat-obs workflow and this runs in the cheap lint lane. YAML validity plus
# the structural assertions below catch every failure mode the template can
# actually produce (unbalanced Jinja, a loop emitting nothing, a job losing its
# relabel block). promtool still validates the rules.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

TEMPLATE="ops/compose/goat-obs/prometheus/prometheus.yml"
[ -r "$TEMPLATE" ] || { echo "missing $TEMPLATE" >&2; exit 1; }

python3 - "$TEMPLATE" "$@" <<'PYEOF'
import re, sys, yaml

try:
    import jinja2
except ImportError:
    print("check-prometheus-template-renders: jinja2 not installed — skipping", file=sys.stderr)
    raise SystemExit(0)

template_path = sys.argv[1]

# The goatnets that ACTUALLY DEPLOY THIS TEMPLATE, which is not the same as
# the goatnets that have isitup_services.
#
# A site whose observability is the `obs-host` role is deliberately absent FROM
# THIS LIST. That role has its OWN roles/obs-host/templates/prometheus.yml.j2 — a
# different file — and nothing runs obs.yml against it. Rendering THIS template
# against its group_vars would "pass" a combination that is never deployed, which
# is worse than not checking it: it reads as coverage.
#
# That role template is checked in its own pass at the bottom of this script.
# It grows a blackbox job when obs_blackbox_enabled is set, and both states are
# asserted below.
#
# The goatnets to render are named on the command line, `--goatnet <name>`
# (repeatable): the ones that deploy this template, which only the deployment
# knows. Each one's group, site_id and site_dns_zone come from its own
# ops/ansible/group_vars/<name>_hosts. With no --goatnet nothing is rendered
# here (the public tree carries no goatnet); the obs-host pass below still runs.
def _group_vars(group):
    out = {}
    base = f"ops/ansible/group_vars/{group}"
    for path in sorted(_glob.glob(base + "/*.yml")) + [p for p in (base + ".yml",) if os.path.isfile(p)]:
        try:
            data = yaml.safe_load(open(path)) or {}
        except yaml.YAMLError:
            continue
        if isinstance(data, dict):
            out.update(data)
    return out


import glob as _glob
import os
GOATNETS = []
_args = sys.argv[2:]
while _args:
    if _args[0] != "--goatnet" or len(_args) < 2:
        print("usage: check-prometheus-template-renders.sh [--goatnet <name>]...", file=sys.stderr)
        raise SystemExit(2)
    _name = _args[1]
    _args = _args[2:]
    _group = _name.replace("-", "_") + "_hosts"
    _gv = _group_vars(_group)
    _site, _zone = _gv.get("site_id"), _gv.get("site_dns_zone")
    if not isinstance(_site, str) or not isinstance(_zone, str) or "{{" in _site + _zone:
        print(f"check-prometheus-template-renders: group_vars/{_group} declares no plain "
              f"site_id and site_dns_zone", file=sys.stderr)
        raise SystemExit(1)
    GOATNETS.append((_name, _group, _site, _zone))

# Jobs that must survive rendering with at least one target. These are the
# ones whose absence is silent: Prometheus starts happily with a job that
# resolved to zero targets, and the only symptom is a panel that reads
# "no data" forever.
MUST_HAVE_TARGETS = ["blackbox-https", "blackbox-surface-app"]


def isitup_services(path, site, zone):
    """Read just the isitup_services block, with group_vars Jinja substituted.

    The surrounding vars.yml is full of Jinja that a plain yaml.safe_load
    chokes on, so the block is sliced textually and the handful of
    interpolations that appear inside it are resolved by hand.
    """
    src = open(path).read()
    i = src.find("\nisitup_services:")
    if i < 0:
        return None
    rest = src[i + 1:]
    m = re.search(r"\n(?=[a-zA-Z_])", rest[len("isitup_services:"):])
    block = rest[: len("isitup_services:") + (m.start() if m else len(rest))]
    block = (block.replace("{{ site_id }}", site)
                  .replace("{{ site_dns_zone }}", zone)
                  .replace("{{ zenoh_stack }}", f"{site}-zenoh"))
    return yaml.safe_load(block)["isitup_services"]


raw = open(template_path).read()
failures = []

import datetime


def _now(utc=False, fmt=None):
    """Ansible's now() global, as the templates call it."""
    d = datetime.datetime.now(datetime.timezone.utc if utc else None)
    return d.strftime(fmt) if fmt else d

def relay_tls_targets(gv_path):
    """Read just the obs_relay_tls_targets list out of a group_vars file.

    Same reason isitup_services() exists: the surrounding vars.yml is full of
    Jinja that a plain safe_load cannot parse. Absent is not an error — a
    goatnet whose relays do not terminate TLS declares none."""
    try:
        text = open(gv_path).read()
    except OSError:
        return []
    m = re.search(r"^obs_relay_tls_targets:\n((?:\s+-\s+.*\n)+)", text, re.M)
    if not m:
        return []
    return yaml.safe_load("obs_relay_tls_targets:\n" + m.group(1))["obs_relay_tls_targets"]


def declared(gv, var):
    """One top-level scalar or plain list from any file of a group_vars directory.

    The Traefik name prefix and the ICMP targets are values the goatnet declares
    and the template only reads. Absent is not an error: the template omits the
    relabel rules and the ICMP job for a goatnet that declares neither."""
    import glob
    for path in sorted(glob.glob(f"ops/ansible/group_vars/{gv}/*.yml")):
        m = re.search(rf"^{re.escape(var)}:[^\n]*\n(?:[ \t]+-[^\n]*\n)*", open(path).read(), re.M)
        if m:
            return yaml.safe_load(m.group(0))[var]
    return None


for name, gv, site, zone in GOATNETS:
    gv_path = f"ops/ansible/group_vars/{gv}/vars.yml"
    legacy_prefix = declared(gv, "obs_traefik_legacy_name_prefix")
    icmp_targets = declared(gv, "obs_blackbox_icmp_targets")
    svcs = isitup_services(gv_path, site, zone)
    if svcs is None:
        failures.append(f"{name}: {gv_path} declares no isitup_services")
        continue

    try:
        tmpl = jinja2.Template(raw, undefined=jinja2.ChainableUndefined)
        rendered = tmpl.render(site_id=site, site_dns_zone=zone,
                               isitup_services=svcs,
                               obs_relay_tls_targets=relay_tls_targets(gv_path),
                               obs_traefik_legacy_name_prefix=legacy_prefix,
                               obs_blackbox_icmp_targets=icmp_targets,
                               portal_traefik_host=f"portal.{site}.{zone}", now=_now)
    except jinja2.TemplateError as e:
        failures.append(f"{name}: template did not render — {e}")
        continue

    try:
        doc = yaml.safe_load(rendered)
    except yaml.YAMLError as e:
        failures.append(f"{name}: rendered output is not valid YAML — {e}")
        continue

    jobs = {j["job_name"]: j for j in doc.get("scrape_configs", [])}
    before = len(failures)
    counts = {}
    for job in MUST_HAVE_TARGETS:
        if job not in jobs:
            failures.append(f"{name}: job {job!r} missing from rendered config")
            continue
        n = sum(len(sc.get("targets") or []) for sc in (jobs[job].get("static_configs") or []))
        counts[job] = n
        if n == 0:
            failures.append(f"{name}: job {job!r} rendered ZERO targets")
        # A blackbox job without the relabel block scrapes the target URL
        # directly instead of the exporter — it "works" and measures nothing.
        if not jobs[job].get("relabel_configs"):
            failures.append(f"{name}: job {job!r} lost its relabel_configs")

    # Relay TLS. NOT in MUST_HAVE_TARGETS on purpose: a goatnet whose
    # relays do not terminate TLS has zero targets and that is CORRECT, so
    # demanding targets everywhere would manufacture a false failure — which is
    # how checks get switched off. What is always required is that the job
    # renders and is wired to the exporter: a declared target that scrapes the
    # relay directly instead of blackbox measures nothing and reports success,
    # the same shape as the certificate expiry nobody was reading.
    RELAY_JOB = "blackbox-relay-tls"
    if RELAY_JOB not in jobs:
        failures.append(f"{name}: job {RELAY_JOB!r} missing from rendered config")
    else:
        n_relay = sum(len(sc.get("targets") or [])
                      for sc in (jobs[RELAY_JOB].get("static_configs") or []))
        counts[RELAY_JOB] = n_relay
        if not jobs[RELAY_JOB].get("relabel_configs"):
            failures.append(f"{name}: job {RELAY_JOB!r} lost its relabel_configs")
        if jobs[RELAY_JOB].get("params", {}).get("module") != ["tls_connect"]:
            failures.append(
                f"{name}: job {RELAY_JOB!r} must use the tls_connect module — "
                "tcp_connect opens a socket and exports no certificate expiry")
        for sc in (jobs[RELAY_JOB].get("static_configs") or []):
            for t in (sc.get("targets") or []):
                if "://" in t or ":" not in t:
                    failures.append(
                        f"{name}: relay target {t!r} must be host:port, not a URL")

    # Declared values the template only reads. A goatnet that declares a
    # Traefik name prefix must get both relabel rules built from it, and one
    # that declares ICMP targets must get exactly those behind the exporter.
    # Without this a renamed variable renders a config with neither and the
    # only symptom is a label that changed value or a panel with no data.
    if legacy_prefix:
        want = f"^{legacy_prefix}-(.*)$"
        got = {r.get("target_label"): r.get("regex")
               for r in (jobs.get("traefik", {}).get("metric_relabel_configs") or [])}
        if got != {"service": want, "router": want}:
            failures.append(f"{name}: traefik job must strip {legacy_prefix!r}- from "
                            f"service and router, rendered {got}")
    if icmp_targets:
        icmp = jobs.get("blackbox-icmp", {})
        got = [t for sc in (icmp.get("static_configs") or []) for t in (sc.get("targets") or [])]
        if got != list(icmp_targets):
            failures.append(f"{name}: blackbox-icmp targets {got} are not the declared {icmp_targets}")
        if not icmp.get("relabel_configs"):
            failures.append(f"{name}: job 'blackbox-icmp' lost its relabel_configs")

    # LIFECYCLE (#1518). Render once more with three rows that must NOT be
    # probed — planned, retiring, listed: false — and one live control row,
    # and assert on both probe planes. This is the gate that proves marking a
    # row in isitup_services is the whole edit that takes it out of alerting.
    def _fixture(sid, **kw):
        row = {"id": sid, "name": sid, "fqdn": f"{sid}.{site}.{zone}",
               "container": sid, "port": 8080, "health_path": "/healthz"}
        row.update(kw)
        return row
    parked = [_fixture("ci-planned", lifecycle="planned"),
              _fixture("ci-retiring", lifecycle="retiring"),
              _fixture("ci-unlisted", listed=False),
              _fixture("ci-quoted-unlisted", listed="false"),
              _fixture("ci-past-retire", retire_after="2000-01-01")]
    control = _fixture("ci-live-control")
    try:
        doc2 = yaml.safe_load(jinja2.Template(raw, undefined=jinja2.ChainableUndefined).render(
            site_id=site, site_dns_zone=zone,
            isitup_services=svcs + parked + [control],
            obs_relay_tls_targets=relay_tls_targets(gv_path),
            portal_traefik_host=f"portal.{site}.{zone}", now=_now))
        jobs2 = {j["job_name"]: j for j in doc2.get("scrape_configs", [])}
        for job in ("blackbox-https", "blackbox-surface-app"):
            targets = [t for sc in (jobs2.get(job, {}).get("static_configs") or [])
                       for t in (sc.get("targets") or [])]
            for row in parked:
                if any(row["id"] in t for t in targets):
                    failures.append(f"{name}: {job} still probes {row['id']} — a row that "
                                    "is planned, retiring or unlisted must not be a target")
            if not any(control["id"] in t for t in targets):
                failures.append(f"{name}: {job} dropped the live control row — the "
                                "lifecycle filter is removing live services")
    except (jinja2.TemplateError, yaml.YAMLError) as e:
        failures.append(f"{name}: lifecycle fixture render failed — {e}")

    # Only claim OK for a goatnet that actually passed. Printing a green line
    # next to a failure is the exact dishonesty this whole change is about.
    verdict = "OK  " if len(failures) == before else "FAIL"
    print(f"  {name:10} {verdict}  " + "  ".join(f"{k}={v}" for k, v in counts.items()))

# NOTE: deliberately no early exit here. A goat-obs failure used to end the
# script, which would now hide the obs-host results entirely — and a partial
# report that looks complete is the failure mode this whole file exists to
# prevent. Both passes run; failures are pooled and reported together below.

# ---------------------------------------------------------------------------
# Second template: the obs-host ROLE's prometheus.yml.j2.
# ---------------------------------------------------------------------------
# A different file for a different stack (a dedicated obs host). Its blackbox job is gated on
# obs_blackbox_enabled, and BOTH states are load-bearing:
#
#   enabled  — the job must exist WITH targets and its relabel block, or
#              probe_ssl_earliest_cert_expiry never appears and the
#              tls-cert-renewal alerts cannot fire: no job, no series, no alerts.
#   disabled — the job must NOT appear. The same rule file carries
#              absent(probe_ssl_earliest_cert_expiry{...}), so a job that
#              leaked onto a goatnet without an exporter would pair a
#              permanently-firing critical with no way to satisfy it.
import glob, os

role_tmpl_path = "ops/ansible/roles/obs-host/templates/prometheus.yml.j2"
role_raw = open(role_tmpl_path).read()

def _regex_replace(v, pattern, repl=""):
    return re.sub(pattern, repl, str(v))

def _unique(seq):
    out = []
    for x in seq:
        if x not in out:
            out.append(x)
    return out

role_env = jinja2.Environment(undefined=jinja2.ChainableUndefined)
role_env.filters["regex_replace"] = _regex_replace
role_env.filters["unique"] = _unique

role_failures = []
role_checked = 0

# The role's registry template (templates/registry-rows.json.j2), rendered the
# way tasks/main.yml does: one {fqdn, lifecycle, listed} per registry row and
# per external-service hostname.
import json as _json
reg_env = jinja2.Environment(undefined=jinja2.StrictUndefined)
reg_env.filters["bool"] = lambda v: (str(v).strip().lower() in ("1", "true", "yes", "on", "y", "t")) if isinstance(v, str) else bool(v)
reg_env.filters["to_json"] = _json.dumps
reg_env.globals["now"] = _now
reg_env.tests["match"] = lambda v, pat: re.match(pat, str(v)) is not None   # Ansible's test
reg_env.tests["mapping"] = lambda v: isinstance(v, dict)
reg_tmpl = reg_env.from_string(open("ops/ansible/roles/obs-host/templates/registry-rows.json.j2").read())


def registry_rows(reg):
    return _json.loads(reg_tmpl.render(reg=reg))


# The registry template itself: a declaration's lifecycle, per-hostname list,
# enabled: false and a past retire_after must all reach the obs host.
_probe = registry_rows({
    "site_id": "s", "site_dns_zone": "z",
    "isitup_services": [{"fqdn": "live.s.z"}, {"fqdn": "old.s.z", "retire_after": "2000-01-01"},
                        {"fqdn": "own.s.z", "lifecycle": "live"}],
    "external_service_a": {"name": "a", "hostnames": ["a", "api.a", "own"],
                           "status": {"lifecycle": "planned", "hostnames": {"api.a": {"list": False}}}},
    "external_service_b": {"name": "b", "enabled": False},
    "external_service_c": {"name": "c", "retire_after": "2000-01-01"},
})
_by = {r["fqdn"]: r for r in _probe}
for fq, want, why in [
        ("live.s.z", (True, "live"), "a plain registry row is live and listed"),
        ("old.s.z", (False, "live"), "a registry row past retire_after is unlisted"),
        ("a.s.z", (True, "planned"), "a declaration's status.lifecycle reaches the obs host"),
        ("api.a.s.z", (False, "planned"), "a per-hostname list: false reaches the obs host"),
        ("own.s.z", (True, "live"), "a hostname an isitup_services row names keeps that row's state"),
        ("b.s.z", (False, "live"), "an enabled: false declaration is unlisted"),
        ("c.s.z", (False, "live"), "a declaration past retire_after is unlisted")]:
    got = _by.get(fq)
    if got is None or (got["listed"], got["lifecycle"]) != want:
        role_failures.append(f"registry-rows.json.j2: {why} — {fq} rendered {got}")

for obs_vars in sorted(glob.glob("ops/ansible/group_vars/*_obs_hosts/vars.yml")):
    group = os.path.basename(os.path.dirname(obs_vars))
    try:
        obs = yaml.safe_load(open(obs_vars)) or {}
    except yaml.YAMLError as e:
        role_failures.append(f"{group}: vars.yml is not valid YAML — {e}")
        continue

    enabled = bool(obs.get("obs_blackbox_enabled"))
    ctx = dict(obs)
    ctx.setdefault("obs_scrape_targets", [])
    ctx.setdefault("obs_blackbox_module", "http_gate")
    ctx["ansible_date_time"] = {"iso8601": "ci"}
    ctx["inventory_hostname"] = group
    ctx["ansible_default_ipv4"] = {"address": "127.0.0.1"}

    # The registry the role filters targets against (#1518). The real one is
    # read through hostvars at deploy; here, from the named group's vars file.
    # Rendered twice: with the registry as it is (every declared probe must
    # survive), and with one extra row marking the FIRST target's host
    # planned (that target, and only that one, must disappear).
    reg_group = obs.get("obs_blackbox_registry_group", "")
    reg_rows = []
    if enabled and reg_group:
        reg_site = yaml.safe_load(open(f"ops/ansible/group_vars/{reg_group}/vars.yml",
                                       encoding="utf-8").read().split("\nisitup_services:")[0]) or {}
        reg = {"site_id": reg_site.get("site_id", ""), "site_dns_zone": reg_site.get("site_dns_zone", ""),
               "isitup_services": isitup_services(f"ops/ansible/group_vars/{reg_group}/vars.yml",
                                                  reg_site.get("site_id", ""),
                                                  reg_site.get("site_dns_zone", "")) or []}
        for decl in sorted(glob.glob(f"ops/ansible/group_vars/{reg_group}/external-service-*.yml")):
            reg.update(yaml.safe_load(open(decl, encoding="utf-8")) or {})
        reg_rows = registry_rows(reg)
    ctx["obs_blackbox_registry_rows"] = reg_rows
    declared_targets = list(obs.get("obs_blackbox_targets") or [])
    if enabled and declared_targets:
        first_host = re.sub(r"[:/].*$", "", re.sub(r"^https?://", "", declared_targets[0]))
        ctx_parked = dict(ctx, obs_blackbox_registry_rows=reg_rows + [
            {"id": "ci-parked", "fqdn": first_host, "lifecycle": "planned"}])
        try:
            doc_p = yaml.safe_load(role_env.from_string(role_raw).render(**ctx_parked))
            jobs_p = {j["job_name"]: j for j in doc_p.get("scrape_configs", [])}
            got = [t for sc in (jobs_p.get("blackbox-https", {}).get("static_configs") or [])
                   for t in (sc.get("targets") or [])]
            if declared_targets[0] in got:
                role_failures.append(f"{group}: a target whose registry row is planned is "
                                     f"still probed ({declared_targets[0]})")
            others = [t for t in declared_targets
                      if re.sub(r"[:/].*$", "", re.sub(r"^https?://", "", t)) != first_host]
            if any(t not in got for t in others):
                role_failures.append(f"{group}: marking one row planned dropped other targets too")
        except (jinja2.TemplateError, yaml.YAMLError) as e:
            role_failures.append(f"{group}: lifecycle fixture render failed — {e}")

    try:
        rendered = role_env.from_string(role_raw).render(**ctx)
        doc = yaml.safe_load(rendered)
    except (jinja2.TemplateError, yaml.YAMLError) as e:
        role_failures.append(f"{group}: obs-host prometheus.yml.j2 did not render — {e}")
        continue

    jobs = {j["job_name"]: j for j in doc.get("scrape_configs", [])}
    role_checked += 1
    before = len(role_failures)

    n_targets = 0
    if enabled:
        job = jobs.get("blackbox-https")
        if job is None:
            role_failures.append(f"{group}: obs_blackbox_enabled is true but no "
                                 "'blackbox-https' job rendered — no cert metric, "
                                 "so tls-cert-renewal cannot fire")
        else:
            n = sum(len(sc.get("targets") or []) for sc in (job.get("static_configs") or []))
            if n == 0:
                role_failures.append(f"{group}: 'blackbox-https' rendered ZERO targets")
            if not job.get("relabel_configs"):
                role_failures.append(f"{group}: 'blackbox-https' lost its relabel_configs — "
                                     "it would scrape the target URL instead of the exporter")
            if not obs.get("obs_blackbox_resolve_ip"):
                role_failures.append(f"{group}: obs_blackbox_resolve_ip unset; mesh-zone probe "
                                     "targets NXDOMAIN inside the container")
            n_targets = n
    else:
        if "blackbox-https" in jobs:
            role_failures.append(f"{group}: blackbox job rendered while "
                                 "obs_blackbox_enabled is false — the tls-cert-renewal "
                                 "absent() guard would fire forever here")

    verdict = "OK  " if len(role_failures) == before else "FAIL"
    state = f"blackbox={'on' if enabled else 'off'}"
    print(f"  {group:24} {verdict}  {state} targets={n_targets}")

failures.extend(role_failures)

if failures:
    print("\ncheck-prometheus-template-renders: FAIL", file=sys.stderr)
    for f in failures:
        print(f"  - {f}", file=sys.stderr)
    raise SystemExit(1)

print(f"check-prometheus-template-renders: OK "
      f"({len(GOATNETS)} goat-obs + {role_checked} obs-host)")
PYEOF
