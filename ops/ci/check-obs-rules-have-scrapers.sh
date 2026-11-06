#!/usr/bin/env bash
# check-obs-rules-have-scrapers.sh — every alert rule a goatnet loads keys on a
# metric family that something in that goatnet actually scrapes.
#
# THE DEFECT THIS PREVENTS. `obs_rule_files` is a role default, so a
# rule file authored for one goatnet loads on every obs host in the fleet.
# `obs_scrape_targets` is a per-goatnet list that starts empty. The fleet-wide
# half grows, the per-goatnet half does not, and nothing else compares them, so
# a goatnet can run alert rules over metric families its Prometheus has never
# been told to scrape.
#
# A rule with no metric is exactly as silent as no rule, and it reads greener:
# an unfed alert and a healthy system are the same colour on every dashboard.
#
# Sibling: check-blackbox-covers-registry.sh does this for PROBE targets
# (a declared service with no probe). This does it for SCRAPE targets (a
# loaded rule with no producer). Same shape, other half of the surface.
#
# Scope: every group_vars/*_obs_hosts/ — the obs-host role's consumers. The
# co-located goat-obs stack is not checked here; its rules and its scrape
# config live in one compose project, which is why it does not have this
# failure mode.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

python3 - <<'PY'
import datetime, glob, os, re, sys

try:
    import yaml
except ImportError:
    print("check-obs-rules-have-scrapers: SKIP — pyyaml not available")
    sys.exit(0)

ROLE_DEFAULTS = "ops/ansible/roles/obs-host/defaults/main.yml"
RULES_DIR     = "ops/compose/goat-obs/prometheus/rules"
SOURCES       = "ops/pins/obs-metric-sources.yaml"

# PromQL keywords, functions and aggregation modifiers. A bare word that is one
# of these is not a metric name. Kept explicit rather than clever: a missed
# entry shows up as a loud "no source declared" failure, never as a silent pass.
PROMQL = {
    "abs", "absent", "absent_over_time", "and", "atan2", "avg", "avg_over_time",
    "bool", "bottomk", "by", "ceil", "changes", "clamp", "clamp_max",
    "clamp_min", "count", "count_over_time", "count_values", "day_of_month",
    "day_of_week", "day_of_year", "days_in_month", "delta", "deriv", "exp",
    "floor", "group", "group_left", "group_right", "histogram_quantile",
    "holt_winters", "hour", "idelta", "ignoring", "increase", "irate",
    "label_join", "label_replace", "last_over_time", "ln", "log10", "log2",
    "max", "max_over_time", "min", "min_over_time", "minute", "month",
    "offset", "on", "or", "predict_linear", "present_over_time", "quantile",
    "quantile_over_time", "rate", "resets", "round", "scalar", "sgn", "sort",
    "sort_desc", "sqrt", "stddev", "stddev_over_time", "stdvar", "sum",
    "sum_over_time", "time", "timestamp", "topk", "unless", "vector", "without",
    "year", "inf", "nan",
}


def norm(job):
    """Job names differ only in punctuation across goatnets.

    zagros-prod declares `node_exporter` and `signal_server`; goat-obs uses
    `node-exporter` and `signal-server`. They are the same job. Comparing raw
    strings would report a gap that does not exist, which trains people to add
    exceptions to a guard rather than fix it.
    """
    return re.sub(r"[-_]", "", str(job).strip().lower())


def load(path):
    with open(path, encoding="utf-8") as fh:
        return yaml.safe_load(fh) or {}


def load_group(group):
    """Every yaml file in a group_vars directory, merged.

    NOT just vars.yml. Ansible loads the whole directory, and this fleet uses
    that: `obs_mesh_pq_enabled` lives in
    group_vars/zagros_prod_obs_hosts/mesh-path-quality.yml, beside the peer list
    it belongs to. Reading only vars.yml sees the role default instead and
    reports a fed rule as unfed — which is the failure mode that makes people
    delete a guard rather than trust it.
    """
    merged = {}
    for path in sorted(glob.glob("ops/ansible/group_vars/%s/*.yml" % group)):
        merged.update(load(path))
    return merged


def metrics_in(rule_file):
    """Metric families a rule file READS, and the ones it defines itself.

    Returns (read, recorded). A recording rule's output is a metric this file
    produces, so a later alert keying on it is fed by definition and must not
    be reported as a gap — bandwidth-attribution.yml records
    `link_utilisation_ratio` and then alerts on it.

    The stripping order below is load-bearing. These exprs carry `#` comments
    and long label matchers, and a naive identifier scan reads the prose in
    them as metric names ("plausible", "precedence", "rather"). Each step
    removes one syntactic construct that can hold a bare word which is not a
    metric.
    """
    doc = load(rule_file)
    read, recorded = set(), set()

    for group in doc.get("groups") or []:
        for rule in group.get("rules") or []:
            if rule.get("record"):
                recorded.add(str(rule["record"]))

            expr = str(rule.get("expr", ""))
            expr = re.sub(r"#[^\n]*", " ", expr)            # PromQL comments
            expr = re.sub(r'"[^"]*"', " ", expr)            # label values
            expr = re.sub(r"'[^']*'", " ", expr)
            expr = re.sub(r"\{[^{}]*\}", " ", expr)         # label matchers
            # Aggregation modifiers take bare LABEL names, not metrics.
            expr = re.sub(r"\b(by|without|on|ignoring|group_left|group_right)\s*\([^()]*\)",
                          " ", expr)

            # Match the WHOLE identifier, then decide. A lookahead glued to the
            # pattern (`...(?!\()`) lets the regex backtrack to a shorter
            # prefix that satisfies it, so `clamp_max(` yields `clamp_ma` — a
            # plausible-looking metric name that is not in PROMQL and gets
            # reported as an undeclared source.
            for m in re.finditer(r"[a-zA-Z_][a-zA-Z0-9_:]*", expr):
                word = m.group(0)
                if word in PROMQL:
                    continue
                if expr[m.end():].lstrip().startswith("("):
                    continue                     # a function call, not a metric
                # Every Prometheus metric family in this fleet is snake_case.
                # `up` is the one single-word metric and is declared in
                # always_available. Requiring an underscore is what keeps
                # leftover prose out without a stopword list to maintain.
                if "_" not in word:
                    continue
                read.add(word)

    return read, recorded


sources_doc   = load(SOURCES)
always        = sources_doc.get("always_available") or []
declared_gaps = sources_doc.get("declared_gaps") or {}
# Longest prefix first so `minio_bucket_` wins over `minio_`.
sources = sorted(
    sources_doc.get("sources") or [],
    key=lambda s: len(str(s.get("prefix", ""))),
    reverse=True,
)


def always_ok(metric):
    return any(metric == a or (a.endswith("_") and metric.startswith(a))
               for a in always)


def source_for(metric):
    for s in sources:
        p = str(s.get("prefix", ""))
        if metric == p or (p.endswith("_") and metric.startswith(p)):
            return s
    return None


role_default_rules = load(ROLE_DEFAULTS).get("obs_rule_files") or []

failures, notes, checked = [], [], 0

for obs_dir in sorted(glob.glob("ops/ansible/group_vars/*_obs_hosts")):
    group = os.path.basename(obs_dir)
    obs   = load_group(group)

    # A gate flag lives on whichever group owns the producer: obs_* keys on
    # the obs group (the timer runs there), everything else on the site
    # group (the producer runs on the mgmt hosts). Look in both.
    site_group = group[: -len("_obs_hosts")] + "_hosts"
    site       = load_group(site_group)
    gates      = dict(site)
    gates.update(obs)
    gate_where = "group_vars/{%s,%s}/" % (site_group, group)

    # Effective rule set — mirrors roles/obs-host/tasks/main.yml's
    # `obs_rule_files_all`: the operator list (or the role default when the
    # operator declares none) plus tls-cert-renewal, which rides the blackbox
    # flag so the rule and its exporter arrive together.
    rule_files = list(obs.get("obs_rule_files") or role_default_rules)
    # obs_rule_files_exclude drops files a site deliberately does not load,
    # mirroring the role's own obs_rule_files_all fact. Checking a rule the
    # site never installs would report a gap that cannot exist.
    excluded   = set(obs.get("obs_rule_files_exclude") or [])
    rule_files = [f for f in rule_files if f not in excluded]
    blackbox   = bool(obs.get("obs_blackbox_enabled"))
    if blackbox and "tls-cert-renewal.yml" not in rule_files:
        rule_files.append("tls-cert-renewal.yml")
    if not rule_files:
        continue

    # Jobs this goatnet's Prometheus will have. Two are emitted unconditionally
    # by prometheus.yml.j2; `obs-host-node` is the obs host's own node_exporter,
    # so it satisfies node_* families even when no mgmt host is scraped.
    jobs = {norm(t.get("job")) for t in (obs.get("obs_scrape_targets") or []) if t.get("job")}
    jobs |= {norm("prometheus"), norm("node-exporter")}
    if blackbox:
        jobs.add(norm("blackbox-https"))

    # A monitoring host can scrape another site's core (a failover
    # arrangement). Read a gated producer's setting from that target's declared
    # owner, not from the monitoring host's local site. Unknown owners fail
    # closed rather than falling back to a convenient local true flag.
    producer_settings = {}
    for target in obs.get("obs_scrape_targets") or []:
        owner = target.get("producer_group")
        if owner is None:
            continue
        if (not isinstance(owner, str)
                or not re.fullmatch(r"[a-zA-Z0-9_]+_hosts", owner)
                or not os.path.isdir("ops/ansible/group_vars/" + owner)):
            failures.append("%s: target %r has invalid producer_group %r"
                            % (group, target.get("name"), owner))
            producer_settings[id(target)] = {}
        else:
            producer_settings[id(target)] = load_group(owner)

    checked += 1

    for rule_file in rule_files:
        path = os.path.join(RULES_DIR, rule_file)
        if not os.path.exists(path):
            # base-alerts.yml is rendered from a role template, not copied from
            # the shared rules dir, so it legitimately has no file here.
            continue

        gap = (declared_gaps.get(group) or {}).get(rule_file)
        read, recorded = metrics_in(path)

        unfed = []
        for metric in sorted(read):
            if always_ok(metric) or metric in recorded:
                continue
            src = source_for(metric)
            if src is None:
                failures.append(
                    "%s: rule file %s references %r and no row in %s declares "
                    "what produces it — add one, or the guard cannot tell a real "
                    "gap from an unknown metric"
                    % (group, rule_file, metric, SOURCES))
                continue

            if norm(src["job"]) not in jobs:
                unfed.append("%s (job %r is not in obs_scrape_targets; %s)"
                             % (metric, src["job"], src.get("produced_by", "")))
                continue

            gate = src.get("requires_var")
            matching_targets = [t for t in (obs.get("obs_scrape_targets") or [])
                                if norm(t.get("job", "")) == norm(src["job"])]
            if gate and gate.startswith("obs_"):
                # These timers run on the observer itself, never on a remote
                # node just because it exposes the same scrape job.
                fed = bool(obs.get(gate))
            elif matching_targets:
                fed = any(producer_settings.get(id(t), gates).get(gate)
                          for t in matching_targets)
            else:
                fed = bool(gates.get(gate))
            if gate and not fed:
                unfed.append("%s (job %r is scraped, but %s is not true in %s, "
                             "or the target producer_group, so nothing writes the series; %s)"
                             % (metric, src["job"], gate, gate_where,
                                src.get("produced_by", "")))

        if not unfed:
            if gap:
                notes.append("%s: declared_gap for %s is now stale — every metric "
                             "it names is fed. Delete the entry."
                             % (group, rule_file))
            continue

        if gap:
            if not str(gap.get("reason", "")).strip():
                failures.append("%s: declared_gap for %s has no reason" % (group, rule_file))
                continue
            # AN EXPIRED GAP IS A FAILURE, NOT A NOTE.
            #
            # The first version of this guard printed the date and never read
            # it, which makes `until:` decoration — the gap sits forever and
            # the date is a comment that reassures whoever added it. The whole
            # point of dating a deviation is that it expires rather than
            # settling in (ops/pins/parity-capabilities.yaml states the same
            # rule, and report.py enforces it for that file).
            #
            # A missing date is also a failure: a gap with no expiry is a
            # permanent exemption wearing a temporary one's clothes.
            until = str(gap.get("until", "")).strip()
            if not until:
                failures.append(
                    "%s: declared_gap for %s has a reason but no `until:` date — "
                    "an undated gap is a permanent exemption, which is what this "
                    "field exists to prevent" % (group, rule_file))
                continue
            try:
                due = datetime.date.fromisoformat(until)
            except ValueError:
                failures.append("%s: declared_gap for %s has `until: %s`, which is not "
                                "an ISO date (YYYY-MM-DD)" % (group, rule_file, until))
                continue
            today = datetime.date.today()
            if due < today:
                failures.append(
                    "%s: declared_gap for %s EXPIRED on %s (%d days ago).\n"
                    "      Close it, or re-date it with what changed and why it needs "
                    "longer. Re-dating is a decision someone makes; silence is not."
                    % (group, rule_file, until, (today - due).days))
                continue
            notes.append("%s: %s declared unfed until %s (%d days left)"
                         % (group, rule_file, until, (due - today).days))
            continue

        failures.append(
            "%s loads %s, but nothing feeds it:\n      - %s"
            % (group, rule_file, "\n      - ".join(unfed)))

for n in notes:
    print("  note: %s" % n)

if failures:
    print("check-obs-rules-have-scrapers: FAIL")
    for f in failures:
        print("  - %s" % f)
    print()
    print("  A loaded rule whose metric nothing scrapes cannot fire, and reads")
    print("  the same green as a healthy system. Either scrape what feeds it")
    print("  (add the job to obs_scrape_targets), stop loading it (drop it from")
    print("  obs_rule_files), or declare the gap with a reason and an `until:`")
    print("  in %s." % SOURCES)
    sys.exit(1)

if checked == 0:
    print("check-obs-rules-have-scrapers: OK — no obs group declares rule files")
else:
    print("check-obs-rules-have-scrapers: PASS — %d obs group(s), every loaded "
          "rule has a producer or a declared gap" % checked)
PY
