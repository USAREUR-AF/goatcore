#!/usr/bin/env python3
"""Generator for the goat-obs Grafana dashboard suite (Block 82B "prove-out").

Spec -> generator -> dashboard JSON, the same idiom as expected-vendors/
generate.py and ops/serve/generate.py. Hand-authoring 11 large Grafana
dashboards is error-prone (one wrong datasource uid = "No data" on every
panel); a generator makes the datasource pin, schemaVersion, and panel
conventions correct-by-construction and trivially re-tunable.

Run:  python3 gen-dashboards.py
Emits one <uid>.json per dashboard into this directory (the dir Grafana
provisions read-only from /var/lib/grafana/dashboards).

Datasource: the single provisioned Prometheus, uid "prometheus"
(grafana/provisioning/datasources/prometheus.yml pins it). Every panel +
target references it explicitly so a fresh install resolves without the
auto-uid hash mismatch the datasource file warns about.

Metric provenance (so panels light up, not "No data"):
  GROUNDED in-repo (prometheus.yml scrape comments / rules / exporters):
    node_*  cadvisor container_*  namedprocess_namegroup_*
    goat_audit_*  goat_expected_vendor  minio_bucket_*
    pg_*  probe_*  up  ALERTS  prometheus_tsdb_*  scrape_*
  DEFENSIVE (metric name not yet referenced anywhere in-repo; built so the
  PromQL is correct IF the series exists, flagged in the design doc):
    traefik_*  (Traefik v3 names — confirm on first live load)
    signal-server app metrics (we lean on `up` + go_* process metrics)
  BRIDGE (new — emitted by ops/relay/health/relay-health-to-prom.py via the
  node-exporter textfile collector; see the relay dashboard):
    goat_relay_*
"""
import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
DS = {"type": "prometheus", "uid": "prometheus"}
SCHEMA_VERSION = 39


# --------------------------------------------------------------------------
# layout helper: pack panels left-to-right, wrapping at the 24-col grid
# --------------------------------------------------------------------------
class Layout:
    def __init__(self):
        self._id = 0
        self.x = 0
        self.y = 0
        self.row_h = 0
        self.panels = []

    def nid(self):
        self._id += 1
        return self._id

    def _place(self, panel, w, h):
        if self.x + w > 24:
            self.x = 0
            self.y += self.row_h
            self.row_h = 0
        panel["gridPos"] = {"w": w, "h": h, "x": self.x, "y": self.y}
        self.x += w
        self.row_h = max(self.row_h, h)
        self.panels.append(panel)
        return panel

    def newline(self):
        self.x = 0
        self.y += self.row_h
        self.row_h = 0

    # ---- panel constructors -------------------------------------------
    def _targets(self, exprs):
        out = []
        for i, e in enumerate(exprs):
            if isinstance(e, tuple):
                expr, legend = e
            else:
                expr, legend = e, ""
            t = {
                "datasource": DS,
                "expr": expr,
                "refId": chr(ord("A") + i),
                "legendFormat": legend,
            }
            out.append(t)
        return out

    def _inst_targets(self, exprs):
        out = self._targets(exprs)
        for t in out:
            t["instant"] = True
            t["format"] = "table"
        return out

    def text(self, title, content, w=24, h=4, mode="markdown"):
        p = {
            "id": self.nid(),
            "type": "text",
            "title": title,
            "options": {"mode": mode, "content": content},
            "transparent": True,
        }
        return self._place(p, w, h)

    def ts(self, title, exprs, unit="short", w=12, h=8, stack=False,
           desc="", legend_table=False, fill=10, minz=None, decimals=None,
           draw="line"):
        custom = {
            "drawStyle": draw,
            "lineWidth": 2,
            "fillOpacity": fill,
            "showPoints": "never",
            "spanNulls": True,
        }
        if stack:
            custom["stacking"] = {"mode": "normal", "group": "A"}
        defaults = {"unit": unit, "custom": custom}
        if minz is not None:
            defaults["min"] = minz
        if decimals is not None:
            defaults["decimals"] = decimals
        opts = {
            "tooltip": {"mode": "multi", "sort": "desc"},
            "legend": {
                "displayMode": "table" if legend_table else "list",
                "placement": "bottom",
                "calcs": ["mean", "lastNotNull", "max"] if legend_table else [],
            },
        }
        p = {
            "id": self.nid(),
            "type": "timeseries",
            "title": title,
            "description": desc,
            "datasource": DS,
            "fieldConfig": {"defaults": defaults, "overrides": []},
            "options": opts,
            "targets": self._targets(exprs),
        }
        return self._place(p, w, h)

    def stat(self, title, exprs, unit="short", w=4, h=4, thresholds=None,
             mappings=None, color="thresholds", text_mode="auto",
             graph=False, desc="", decimals=None, reducer="lastNotNull"):
        defaults = {"unit": unit, "color": {"mode": color}}
        if thresholds is not None:
            defaults["thresholds"] = {"mode": "absolute", "steps": thresholds}
        else:
            defaults["thresholds"] = {"mode": "absolute",
                                      "steps": [{"color": "text", "value": None}]}
        if mappings:
            defaults["mappings"] = mappings
        if decimals is not None:
            defaults["decimals"] = decimals
        p = {
            "id": self.nid(),
            "type": "stat",
            "title": title,
            "description": desc,
            "datasource": DS,
            "fieldConfig": {"defaults": defaults, "overrides": []},
            "options": {
                "reduceOptions": {"calcs": [reducer], "fields": "", "values": False},
                "textMode": text_mode,
                "colorMode": "value",
                "graphMode": "area" if graph else "none",
                "justifyMode": "auto",
                "orientation": "auto",
            },
            "targets": self._targets(exprs),
        }
        return self._place(p, w, h)

    def gauge(self, title, exprs, unit="percent", w=4, h=6, minv=0, maxv=100,
              thresholds=None, desc="", decimals=0):
        if thresholds is None:
            thresholds = [{"color": "green", "value": None},
                          {"color": "yellow", "value": 70},
                          {"color": "red", "value": 90}]
        defaults = {
            "unit": unit, "min": minv, "max": maxv, "decimals": decimals,
            "color": {"mode": "thresholds"},
            "thresholds": {"mode": "absolute", "steps": thresholds},
        }
        p = {
            "id": self.nid(),
            "type": "gauge",
            "title": title,
            "description": desc,
            "datasource": DS,
            "fieldConfig": {"defaults": defaults, "overrides": []},
            "options": {
                "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
                "showThresholdLabels": False, "showThresholdMarkers": True,
            },
            "targets": self._targets(exprs),
        }
        return self._place(p, w, h)

    def bargauge(self, title, exprs, unit="short", w=12, h=8, thresholds=None,
                 desc="", orientation="horizontal", maxv=None):
        if thresholds is None:
            thresholds = [{"color": "green", "value": None}]
        defaults = {"unit": unit, "color": {"mode": "continuous-GrYlRd"},
                    "thresholds": {"mode": "absolute", "steps": thresholds}}
        if maxv is not None:
            defaults["max"] = maxv
        p = {
            "id": self.nid(),
            "type": "bargauge",
            "title": title,
            "description": desc,
            "datasource": DS,
            "fieldConfig": {"defaults": defaults, "overrides": []},
            "options": {
                "reduceOptions": {"calcs": ["lastNotNull"], "fields": "", "values": False},
                "orientation": orientation, "displayMode": "gradient",
                "showUnfilled": True,
            },
            "targets": self._targets(exprs),
        }
        return self._place(p, w, h)

    def table(self, title, exprs, w=12, h=8, desc="", overrides=None,
              transformations=None):
        p = {
            "id": self.nid(),
            "type": "table",
            "title": title,
            "description": desc,
            "datasource": DS,
            "fieldConfig": {"defaults": {"custom": {"align": "auto",
                            "filterable": True}}, "overrides": overrides or []},
            "options": {"showHeader": True, "footer": {"show": False}},
            "targets": self._inst_targets(exprs),
            "transformations": transformations or [
                {"id": "organize", "options": {"excludeByName": {
                    "Time": True, "job": True, "instance": True}}}],
        }
        return self._place(p, w, h)

    def timeline(self, title, exprs, w=24, h=8, desc="", mappings=None,
                 thresholds=None):
        if mappings is None:
            mappings = [{"type": "value", "options": {
                "0": {"text": "DOWN", "color": "red", "index": 0},
                "1": {"text": "UP", "color": "green", "index": 1}}}]
        if thresholds is None:
            thresholds = [{"color": "red", "value": None},
                          {"color": "green", "value": 1}]
        p = {
            "id": self.nid(),
            "type": "state-timeline",
            "title": title,
            "description": desc,
            "datasource": DS,
            "fieldConfig": {"defaults": {
                "custom": {"fillOpacity": 80, "lineWidth": 0},
                "mappings": mappings,
                "color": {"mode": "thresholds"},
                "thresholds": {"mode": "absolute", "steps": thresholds},
            }, "overrides": []},
            "options": {"mergeValues": True, "showValue": "never",
                        "rowHeight": 0.9, "legend": {"displayMode": "list",
                        "placement": "bottom"}},
            "targets": self._targets(exprs),
        }
        return self._place(p, w, h)

    def heatmap(self, title, expr, w=12, h=8, desc="", unit="s"):
        p = {
            "id": self.nid(),
            "type": "heatmap",
            "title": title,
            "description": desc,
            "datasource": DS,
            "options": {"calculate": False, "cellGap": 1,
                        "color": {"scheme": "Spectral", "mode": "scheme",
                                  "steps": 64},
                        "yAxis": {"unit": unit}},
            "targets": [{"datasource": DS, "expr": expr, "refId": "A",
                         "format": "heatmap", "legendFormat": "{{le}}"}],
        }
        return self._place(p, w, h)


# The filesystem holding container + database data differs per goatnet, so the
# forecast panels select it rather than naming one. A goatnet can put Docker and
# containerd on a separate NVMe at /opt/docker-data; goatnets without
# that split keep it on /.
_MOUNT_Q = ('label_values(node_filesystem_avail_bytes'
            '{fstype!~"tmpfs|overlay|squashfs|ramfs|devtmpfs"}, mountpoint)')
MOUNT_VAR = {
    "name": "mount",
    "label": "Data mount",
    "type": "query",
    "datasource": {"type": "prometheus", "uid": "prometheus"},
    "definition": _MOUNT_Q,
    "query": {"query": _MOUNT_Q, "refId": "mount"},
    "description": ("The filesystem this goatnet keeps container and database "
                    "data on."),
    "refresh": 1, "sort": 1, "multi": False, "includeAll": False,
    "current": {}, "options": [], "hide": 0, "skipUrlSync": False,
}


def dashboard(uid, title, tags, lay, refresh="30s", time_from="now-6h",
              links_to=None, description="", templating=None):
    links = []
    if links_to:
        links.append({
            "title": "Dashboards", "type": "dashboards", "asDropdown": True,
            "icon": "external link", "includeVars": False, "keepTime": True,
            "tags": ["goat-obs"], "targetBlank": False,
        })
    return {
        "uid": uid,
        "title": title,
        "description": description,
        "tags": tags,
        "schemaVersion": SCHEMA_VERSION,
        "version": 1,
        "editable": True,
        "graphTooltip": 1,
        "refresh": refresh,
        "time": {"from": time_from, "to": "now"},
        "timezone": "",
        "links": links,
        "annotations": {"list": [{
            "builtIn": 1, "datasource": {"type": "grafana", "uid": "-- Grafana --"},
            "enable": True, "hide": True, "name": "Annotations & Alerts",
            "type": "dashboard"}]},
        "templating": {"list": templating or []},
        "panels": lay.panels,
    }


# ==========================================================================
# 1. Relay Fleet Health  (the replication of the custom relay-health dash)
# ==========================================================================
def d_relay():
    L = Layout()
    L.text("", (
        "### Relay Fleet Health — Grafana mirror of `ops/relay/health/`\n"
        "Renders the **same signals** the custom single-file dashboard shows, by "
        "consuming the prober's own output via the node-exporter **textfile "
        "collector** (`relay-health-to-prom.py` → `goat_relay_*`). This is how "
        "Grafana reaches near-parity: it reuses the prober's STUN / enroll-ratio / "
        "deep metrics rather than re-deriving them with blackbox (which cannot do "
        "STUN or the enroll-SNI handshake). Honesty: a stale `.prom` greys via "
        "`goat_relay_probe_timestamp_seconds` age; rates show `—` until two samples."
    ), h=4)

    L.stat("Relays OK", [("sum(goat_relay_up)", "")], unit="short", w=4, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "green", "value": 1}],
           text_mode="value", desc="rathole-control-up count (tunnel-endpoint vantage)")
    L.stat("Fleet size", [("count(goat_relay_up)", "")], w=4, h=4, text_mode="value")
    L.stat("Probe age", [("time() - max(goat_relay_probe_timestamp_seconds)", "")],
           unit="s", w=4, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 60},
                       {"color": "red", "value": 300}],
           desc="seconds since the prober last wrote — staleness honesty")
    L.stat("Worst enroll ratio", [("min(goat_relay_enroll_ok_ratio)", "")],
           unit="percentunit", w=4, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 0.5},
                       {"color": "green", "value": 0.99}],
           desc="lowest :443 ok-ratio across the fleet")
    L.stat("STUN ok", [("sum(goat_relay_stun_ok)", "")], w=4, h=4, text_mode="value",
           desc=":3478 coturn binding-request successes")
    L.stat("Tunnel vantage", [("max(goat_relay_vantage_tunnel_endpoint)", "")],
           w=4, h=4, mappings=[{"type": "value", "options": {
               "1": {"text": "Thor (hairpin)", "color": "yellow"},
               "0": {"text": "external", "color": "green"}}}], text_mode="value",
           desc="on a tunnel-endpoint vantage :443/STUN are informational")

    L.timeline("Per-relay control reachability (:2333 rathole)",
               [("goat_relay_rathole_ctl_up", "{{relay}}")], h=7,
               desc="the non-hairpin health driver on a Thor vantage")
    L.timeline("Per-relay enrollment up (:443, ok-ratio>0)",
               [("goat_relay_enroll_up", "{{relay}}")], h=7)

    L.ts("Enrollment ok-ratio per relay (:443)",
         [("goat_relay_enroll_ok_ratio", "{{relay}}")], unit="percentunit",
         w=12, h=8, minz=0, legend_table=True, fill=0,
         desc="keeps the per-connection flap visible, not a binary up/down")
    L.ts("Enroll TCP-connect latency (ms)",
         [("goat_relay_enroll_tcp_latency_ms", "{{relay}}")], unit="ms",
         w=12, h=8, legend_table=True, fill=0)

    L.table("Fleet snapshot", [
        ("goat_relay_up", ""),
        ("goat_relay_enroll_ok_ratio", ""),
        ("goat_relay_enroll_tcp_latency_ms", ""),
        ("goat_relay_stun_ok", ""),
        ("goat_relay_peers_5m", ""),
    ], w=24, h=8, desc="per-relay current state (instant)")

    L.text("Deep metrics (laptop `--deep` only; greyed when absent)", "", h=1, mode="html")
    L.ts("Relay throughput (rx/tx bytes/s)", [
        ("goat_relay_rx_bps", "rx {{relay}}"),
        ("goat_relay_tx_bps", "tx {{relay}}")], unit="Bps", w=12, h=7, legend_table=True)
    L.ts("Active peers (5m, distinct PeerIds)",
         [("goat_relay_peers_5m", "{{relay}}")], w=6, h=7, draw="bars", fill=60)
    L.ts("goat-relay CPU / mem %", [
        ("goat_relay_cpu_pct", "cpu {{relay}}"),
        ("goat_relay_mem_pct", "mem {{relay}}")], unit="percent", w=6, h=7)

    return dashboard("relay-fleet-health", "Relay Fleet Health",
                     ["goat-obs", "relay", "blackbox"], L, links_to=True,
                     description="Grafana mirror of the custom relay-health dashboard via the textfile-collector bridge.")


# ==========================================================================
# 2. Goatnet Service SLO & Uptime  (blackbox-https / dns)
# ==========================================================================
def d_slo():
    L = Layout()
    L.text("", (
        "### Service SLO & Uptime — blackbox HTTPS / DNS\n"
        "Availability over the dashboard window (`avg_over_time(probe_success)`), "
        "TLS-cert expiry countdown, the HTTP phase-latency breakdown, and DNS "
        "resolver answer-health. Grafana strengths on show: availability math, "
        "`probe_ssl_earliest_cert_expiry` days-left thresholds, stacked phase latency."), h=4)

    L.stat("Endpoints up", [('sum(probe_success{job="blackbox-https"})', "")],
           w=4, h=4, text_mode="value")
    L.stat("Endpoints total", [('count(probe_success{job="blackbox-https"})', "")],
           w=4, h=4, text_mode="value")
    L.stat("Worst availability (window)",
           [('min(avg_over_time(probe_success{job="blackbox-https"}[$__range]))', "")],
           unit="percentunit", w=4, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 0.99},
                       {"color": "green", "value": 0.999}])
    L.stat("Soonest cert expiry",
           [('min(probe_ssl_earliest_cert_expiry{job="blackbox-https"} - time())', "")],
           unit="s", w=4, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 604800},
                       {"color": "green", "value": 2592000}],
           desc="LE certs auto-renew ~30d out; red = under a week")
    L.stat("DNS resolvers answering",
           [('sum(probe_success{job="blackbox-dns"})', "")], w=4, h=4, text_mode="value")
    L.stat("ICMP relays reachable",
           [('sum(probe_success{job="blackbox-icmp"})', "")], w=4, h=4, text_mode="value")

    L.timeline("Endpoint up/down", [('probe_success{job="blackbox-https"}', "{{instance}}")], h=8)

    L.bargauge("Availability over window (per endpoint)",
               [('avg_over_time(probe_success{job="blackbox-https"}[$__range])', "{{instance}}")],
               unit="percentunit", w=12, h=8, maxv=1,
               thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 0.99},
                           {"color": "green", "value": 0.999}])
    L.bargauge("TLS cert days remaining",
               [('(probe_ssl_earliest_cert_expiry{job="blackbox-https"} - time()) / 86400', "{{instance}}")],
               unit="d", w=12, h=8,
               thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 7},
                           {"color": "green", "value": 21}])

    L.ts("Probe duration per endpoint",
         [('probe_duration_seconds{job="blackbox-https"}', "{{instance}}")],
         unit="s", w=12, h=8, legend_table=True)
    L.ts("HTTP phase latency (resolve / connect / tls / processing)",
         [('avg by (phase) (probe_http_duration_seconds{job="blackbox-https"})', "{{phase}}")],
         unit="s", w=12, h=8, stack=True,
         desc="where the time goes — DNS vs TCP vs TLS vs server")

    L.table("HTTP status codes", [
        ('probe_http_status_code{job="blackbox-https"}', ""),
        ('probe_http_version{job="blackbox-https"}', ""),
        ('probe_ssl_earliest_cert_expiry{job="blackbox-https"} - time()', ""),
    ], w=12, h=8)
    L.ts("DNS lookup time per resolver",
         [('probe_dns_lookup_time_seconds{job="blackbox-dns"}', "{{instance}}")],
         unit="s", w=12, h=8)

    return dashboard("goatnet-service-slo", "Service SLO & Uptime",
                     ["goat-obs", "blackbox", "slo"], L, links_to=True,
                     description="Availability, TLS expiry, latency phases, DNS health from blackbox probes.")


# ==========================================================================
# 3. Edge HTTP — RED + Latency  (traefik)  [DEFENSIVE metric names]
# ==========================================================================
def d_edge():
    L = Layout()
    L.text("", (
        "### Edge HTTP — RED method + latency (Traefik)\n"
        "Rate / Errors / Duration on the public edge. **Metric names are Traefik "
        "v3 defaults and are not yet referenced elsewhere in-repo — confirm on "
        "first live load** (`/metrics` on the traefik job). If your build labels "
        "differ, only the queries here change. Grafana strength on show: "
        "`histogram_quantile` percentiles + a latency heatmap."), h=4)

    L.stat("Requests/s (edge)",
           [('sum(rate(traefik_entrypoint_requests_total[5m]))', "")], unit="reqps",
           w=4, h=4, graph=True)
    L.stat("5xx ratio",
           [('sum(rate(traefik_service_requests_total{code=~"5.."}[5m])) / '
             'clamp_min(sum(rate(traefik_service_requests_total[5m])),1)', "")],
           unit="percentunit", w=4, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 0.01},
                       {"color": "red", "value": 0.05}])
    L.stat("4xx ratio",
           [('sum(rate(traefik_service_requests_total{code=~"4.."}[5m])) / '
             'clamp_min(sum(rate(traefik_service_requests_total[5m])),1)', "")],
           unit="percentunit", w=4, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 0.1}])
    L.stat("p99 latency",
           [('histogram_quantile(0.99, sum by (le) (rate(traefik_service_request_duration_seconds_bucket[5m])))', "")],
           unit="s", w=4, h=4)
    L.stat("Open connections",
           [('sum(traefik_open_connections)', "")], w=4, h=4)
    L.stat("Traefik up", [('up{job="traefik"}', "")], w=4, h=4,
           mappings=[{"type": "value", "options": {
               "1": {"text": "UP", "color": "green"}, "0": {"text": "DOWN", "color": "red"}}}],
           text_mode="value")

    L.ts("Request rate by router",
         [('sum by (router) (rate(traefik_router_requests_total[5m]))', "{{router}}")],
         unit="reqps", w=12, h=8, stack=True, legend_table=True)
    L.ts("Status-code class rate",
         [('sum by (code) (rate(traefik_service_requests_total[5m]))', "{{code}}")],
         unit="reqps", w=12, h=8, stack=True)

    L.ts("Latency percentiles (service)", [
        ('histogram_quantile(0.50, sum by (le) (rate(traefik_service_request_duration_seconds_bucket[5m])))', "p50"),
        ('histogram_quantile(0.90, sum by (le) (rate(traefik_service_request_duration_seconds_bucket[5m])))', "p90"),
        ('histogram_quantile(0.99, sum by (le) (rate(traefik_service_request_duration_seconds_bucket[5m])))', "p99"),
    ], unit="s", w=12, h=8)
    L.heatmap("Latency distribution heatmap",
              'sum by (le) (rate(traefik_service_request_duration_seconds_bucket[5m]))',
              w=12, h=8, unit="s")

    L.table("Per-service request rate + error ratio", [
        ('sum by (service) (rate(traefik_service_requests_total[5m]))', ""),
        ('sum by (service) (rate(traefik_service_requests_total{code=~"5.."}[5m]))', ""),
    ], w=24, h=8)

    return dashboard("edge-http-red", "Edge HTTP — RED + Latency",
                     ["goat-obs", "traefik", "edge"], L, links_to=True,
                     description="RED method + latency percentiles for the Traefik edge (metric names defensive — confirm live).")


# ==========================================================================
# 4. Postgres — Connections, Throughput & Storage  (postgres-exporter)
# ==========================================================================
def d_postgres():
    L = Layout()
    L.text("", (
        "### Postgres — connections, throughput & storage\n"
        "The mgmt-plane datastore (NetBird/Ory/portal). Connection saturation vs "
        "`max_connections`, cache-hit ratio, transaction + tuple throughput, "
        "deadlocks, and per-DB size. Grafana strength: gauges for saturation, "
        "ratio math, per-`datname` breakdown."), h=4)

    L.stat("Postgres up", [('pg_up', "")], w=4, h=4, text_mode="value",
           mappings=[{"type": "value", "options": {
               "1": {"text": "UP", "color": "green"}, "0": {"text": "DOWN", "color": "red"}}}])
    L.gauge("Connection saturation",
            [('sum(pg_stat_database_numbackends) / max(pg_settings_max_connections) * 100', "")],
            unit="percent", w=4, h=6,
            thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 70},
                        {"color": "red", "value": 90}])
    L.gauge("Cache hit ratio",
            [('sum(rate(pg_stat_database_blks_hit[5m])) / '
              'clamp_min(sum(rate(pg_stat_database_blks_hit[5m])) + '
              'sum(rate(pg_stat_database_blks_read[5m])),1) * 100', "")],
            unit="percent", w=4, h=6,
            thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 95},
                        {"color": "green", "value": 99}])
    L.stat("Largest DB",
           [('max(pg_database_size_bytes)', "")], unit="bytes", w=4, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 50*1024**3}])
    L.stat("Deadlocks/min",
           [('sum(rate(pg_stat_database_deadlocks[5m])) * 60', "")], w=4, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}])
    L.stat("Commits/s",
           [('sum(rate(pg_stat_database_xact_commit[5m]))', "")], unit="ops", w=4, h=4, graph=True)

    L.ts("Connections (active backends)",
         [('sum(pg_stat_database_numbackends)', "backends"),
          ('max(pg_settings_max_connections)', "max_connections")], w=12, h=8)
    L.ts("Transactions/s (commit vs rollback)", [
        ('sum(rate(pg_stat_database_xact_commit[5m]))', "commit"),
        ('sum(rate(pg_stat_database_xact_rollback[5m]))', "rollback")], unit="ops", w=12, h=8)

    L.ts("Tuple throughput", [
        ('sum(rate(pg_stat_database_tup_fetched[5m]))', "fetched"),
        ('sum(rate(pg_stat_database_tup_inserted[5m]))', "inserted"),
        ('sum(rate(pg_stat_database_tup_updated[5m]))', "updated"),
        ('sum(rate(pg_stat_database_tup_deleted[5m]))', "deleted")], unit="ops", w=12, h=8, legend_table=True)
    L.bargauge("Database sizes", [('pg_database_size_bytes', "{{datname}}")],
               unit="bytes", w=12, h=8)

    return dashboard("postgres-health", "Postgres — Connections, Throughput & Storage",
                     ["goat-obs", "postgres"], L, links_to=True,
                     description="Postgres connection saturation, cache ratio, throughput, storage.")


# ==========================================================================
# 5. Fabric Tenants & Silent-Denial Watch  (audit-subscriber + expected-vendor)
# ==========================================================================
def d_fabric_tenants():
    L = Layout()
    L.text("", (
        "### Fabric Tenants & Silent-Denial Watch\n"
        "Distinct from the *GOAT Audit Subscriber* board (which watches the "
        "subscriber pipeline). This is the **tenant COP**: who is producing, how "
        "much, and the F-177 expected-vs-actual reconciliation — an expected-active "
        "vendor with zero samples is a likely silent denial; a producing vendor "
        "absent from the manifest is a squat. Grafana strength: bar-gauge "
        "leaderboard + the `goat_expected_vendor unless …` join rendered as a table."), h=4)

    L.stat("Producing vendors (15m)",
           [('count(sum by (vendor) (increase(goat_audit_samples_total[15m])) > 0)', "")],
           w=4, h=4, text_mode="value")
    L.stat("Expected-active vendors",
           [('count(goat_expected_vendor{active="true"})', "")], w=4, h=4, text_mode="value")
    L.stat("SILENT (expected, 0 samples)",
           [('count(goat_expected_vendor{active="true"} == 1 unless on(vendor) '
             '(sum by (vendor) (increase(goat_audit_samples_total[15m])) > 0))', "")],
           w=4, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}],
           desc="F-177 silent-denial detector — non-zero is a problem")
    L.stat("SQUAT (producing, unexpected)",
           [('count(sum by (vendor) (rate(goat_audit_samples_total[10m])) > 0 '
             'unless on(vendor) goat_expected_vendor)', "")],
           w=4, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 1}])
    L.stat("Total samples/s",
           [('sum(rate(goat_audit_samples_total[5m]))', "")], unit="ops", w=4, h=4, graph=True)
    L.stat("Total bytes/s",
           [('sum(rate(goat_audit_sample_bytes_total[5m]))', "")], unit="Bps", w=4, h=4, graph=True)

    L.bargauge("Top tenants by sample rate (5m)",
               [('topk(15, sum by (vendor) (rate(goat_audit_samples_total[5m])))', "{{vendor}}")],
               unit="ops", w=12, h=10)
    L.bargauge("Top tenants by byte rate (5m)",
               [('topk(15, sum by (vendor) (rate(goat_audit_sample_bytes_total[5m])))', "{{vendor}}")],
               unit="Bps", w=12, h=10)

    L.table("SILENT — expected-active, zero samples in 15m", [
        ('goat_expected_vendor{active="true"} == 1 unless on(vendor) '
         '(sum by (vendor) (increase(goat_audit_samples_total[15m])) > 0)', ""),
    ], w=12, h=8, desc="each row = a vendor to triage")
    L.table("SQUAT — producing but not in manifest", [
        ('sum by (vendor) (rate(goat_audit_samples_total[10m])) > 0 '
         'unless on(vendor) goat_expected_vendor', ""),
    ], w=12, h=8)

    L.ts("Sample rate by topic-family (key_pattern)",
         [('sum by (key_pattern) (rate(goat_audit_samples_total[5m]))', "{{key_pattern}}")],
         unit="ops", w=24, h=8, stack=True, legend_table=True)

    return dashboard("fabric-tenants", "Fabric Tenants & Silent-Denial Watch",
                     ["goat-obs", "zenoh", "fabric"], L, links_to=True,
                     description="Tenant leaderboard + F-177 expected-vs-actual reconciliation.")


# ==========================================================================
# 6. Audit Integrity & MinIO Storage  (minio + audit s3 + quota)
# ==========================================================================
def d_storage():
    L = Layout()
    L.text("", (
        "### Audit Integrity & MinIO Storage\n"
        "The compliance-grade storage board (ADR 0811). Bucket usage vs configured "
        "quota with a **`predict_linear` days-to-full forecast**, oldest-object age, "
        "and the audit-subscriber S3 put success/fail rate. "
        "Grafana strength: forecasting + ratio gauges + the put-fail "
        "early-warning that took 4 days to notice by hand."), h=4)

    L.gauge("audit-integrity quota used",
            [('100 * minio_bucket_usage_total_bytes{bucket="audit-integrity"} / on(bucket) group_left() minio_bucket_quota_total_bytes{bucket="audit-integrity"}', "")],
            unit="percent", w=5, h=6,
            thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 85},
                        {"color": "red", "value": 95}])
    L.stat("Days to quota-full (forecast)",
           [('(minio_bucket_quota_total_bytes{bucket="audit-integrity"} - '
             'minio_bucket_usage_total_bytes{bucket="audit-integrity"}) / '
             'clamp_min(deriv(minio_bucket_usage_total_bytes{bucket="audit-integrity"}[6h]), 1) / 86400', "")],
           unit="d", w=5, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 7},
                       {"color": "green", "value": 30}],
           desc="predict_linear-style runway from 6h growth")
    L.stat("S3 put fail/s (audit)",
           [('rate(goat_audit_write_errors_total[5m])', "")], unit="ops",
           w=5, h=4, thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 0.001}],
           desc="non-zero = audit events being dropped right now")
    L.stat("S3 put ok/s",
           [('rate(goat_audit_batches_uploaded_total[5m])', "")], unit="ops", w=5, h=4, graph=True)

    L.ts("Bucket usage vs quota (audit-integrity)", [
        ('minio_bucket_usage_total_bytes{bucket="audit-integrity"}', "usage"),
        ('minio_bucket_quota_total_bytes{bucket="audit-integrity"}', "quota")],
        unit="bytes", w=12, h=8,
        desc="watch the gap close; the forecast stat reads this slope")
    L.ts("Audit S3 puts (ok vs failed)", [
        ('rate(goat_audit_batches_uploaded_total[5m])', "ok"),
        ('rate(goat_audit_write_errors_total[5m])', "failed")],
        unit="ops", w=12, h=8)

    L.bargauge("Per-bucket usage", [('minio_bucket_usage_total_bytes', "{{bucket}}")],
               unit="bytes", w=12, h=8)
    L.bargauge("Per-bucket object count",
               [('minio_bucket_usage_object_total', "{{bucket}}")], w=12, h=8)

    L.ts("Audit events received vs batches uploaded", [
        ('rate(goat_audit_samples_total[5m])', "events received"),
        ('rate(goat_audit_batches_uploaded_total[5m])', "batches uploaded")],
        unit="ops", w=24, h=8)

    return dashboard("audit-storage-integrity", "Audit Integrity & MinIO Storage",
                     ["goat-obs", "minio", "audit", "compliance"], L, links_to=True,
                     description="MinIO bucket usage/quota forecast + audit-subscriber S3 health (ADR 0811, F-169, F-171).")


# ==========================================================================
# 7. Container Restart & OOM Watch  (cadvisor)  — incident-focused
# ==========================================================================
def d_containers():
    L = Layout()
    L.text("", (
        "### Container Restart & OOM Watch\n"
        "Operational-incident cut of cAdvisor (distinct from the metrics-browse "
        "*cadvisor* board): what's **flapping** (start-time changes), what's near "
        "its **memory limit** (OOM proximity), what's being **CPU-throttled**, and "
        "what's **running-but-unhealthy** (docker HEALTHCHECK verdicts via the "
        "goat-obs textfile publisher — the F-248/F-253 hang class that restarts "
        "and OOM counters can't see). Grafana strength: `changes()` restart "
        "detection, working-set/limit ratio, a health state-timeline."), h=4)

    L.stat("Containers running",
           [('count(container_last_seen * on (id) group_left(name) goat_container_info)', "")], w=3, h=4, text_mode="value")
    # Two restart tiles, deliberately. "All" is what cAdvisor observed;
    # "unplanned" subtracts the restarts an actor declared through
    # ops/scripts/goat-declare-restart.sh. Planned migration work can produce
    # dozens of restarts, and the all-restarts number has no way to say so.
    # Unplanned is the number to react to.
    L.stat("Restarts (24h) — all",
           [('sum(changes(container_start_time_seconds[24h]) * on (id) group_left(name) goat_container_info)', "")], w=3, h=4,
           desc="every restart cAdvisor saw, deliberate or not",
           thresholds=[{"color": "text", "value": None}], text_mode="value")
    L.stat("Restarts (24h) — unplanned",
           [('clamp_min(sum(changes(container_start_time_seconds[24h]) '
             '* on (id) group_left(name) goat_container_info) '
             '- sum(increase(goat_declared_restart_total[24h])), 0)', "")], w=4, h=4,
           desc="observed restarts minus those declared deliberate",
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 1},
                       {"color": "red", "value": 5}], text_mode="value")
    L.stat("Near mem-limit (>85%)",
           [('count(((container_memory_working_set_bytes / (container_spec_memory_limit_bytes > 0)) '
             '* on (id) group_left(name) goat_container_info) > 0.85) or vector(0)', "")],
           w=3, h=4, thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}],
           text_mode="value")
    L.stat("Throttled (5m)",
           [('count((rate(container_cpu_cfs_throttled_seconds_total[5m]) * on (id) group_left(name) goat_container_info) > 0) or vector(0)', "")],
           w=3, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 1}])
    L.stat("Total container CPU",
           [('sum(rate(container_cpu_usage_seconds_total[5m]) * on (id) group_left(name) goat_container_info)', "")], unit="short",
           w=3, h=4, graph=True, desc="cores")
    L.stat("Total container mem",
           [('sum(container_memory_working_set_bytes * on (id) group_left(name) goat_container_info)', "")], unit="bytes", w=4, h=4)

    # -- healthcheck verdicts (goat_container_health_status, textfile
    #    publisher — F-248/F-253). Only containers that define a docker
    #    HEALTHCHECK appear; the */5 cron bounds freshness at ~5m.
    L.stat("Unhealthy (healthcheck)",
           [('count(goat_container_health_status{status="unhealthy"}) or vector(0)', "")],
           w=4, h=8, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}],
           desc="running-but-unhealthy containers right now")
    L.table("Healthcheck status now", [
        ('goat_container_health_status', ""),
    ], w=8, h=8, desc="docker HEALTHCHECK verdict per container (textfile publisher, ~5m freshness)",
       transformations=[{"id": "organize", "options": {"excludeByName": {
           "Time": True, "Value": True, "__name__": True, "job": True, "instance": True}}}])
    L.timeline("Healthcheck verdict by container", [
        ('max by (name) ((2 * goat_container_health_status{status="unhealthy"}) '
         'or (1 * goat_container_health_status{status="starting"}) '
         'or (0 * goat_container_health_status{status="healthy"}))', "{{name}}")],
        w=12, h=8,
        desc="worst verdict per container over time — unhealthy episodes band red",
        mappings=[{"type": "value", "options": {
            "0": {"text": "HEALTHY", "color": "green", "index": 0},
            "1": {"text": "STARTING", "color": "yellow", "index": 1},
            "2": {"text": "UNHEALTHY", "color": "red", "index": 2}}}],
        thresholds=[{"color": "green", "value": None},
                    {"color": "yellow", "value": 1},
                    {"color": "red", "value": 2}])

    L.table("Restart leaderboard (24h)", [
        ('sort_desc(sum by (name) (changes(container_start_time_seconds[24h]) * on (id) group_left(name) goat_container_info))', ""),
    ], w=12, h=9, desc="containers ordered by restart count")
    L.table("Memory vs limit (OOM proximity)", [
        ('sort_desc(sum by (name) ((container_memory_working_set_bytes / (container_spec_memory_limit_bytes > 0)) '
         '* on (id) group_left(name) goat_container_info))', ""),
    ], w=12, h=9, desc="ratio → 1.0 means about to OOM")

    L.ts("Per-container CPU (cores)",
         [('sum by (name) (rate(container_cpu_usage_seconds_total[5m]) * on (id) group_left(name) goat_container_info)', "{{name}}")],
         unit="short", w=12, h=8, stack=True, legend_table=True)
    L.ts("Per-container working-set memory",
         [('sum by (name) (container_memory_working_set_bytes * on (id) group_left(name) goat_container_info)', "{{name}}")],
         unit="bytes", w=12, h=8, stack=True, legend_table=True)

    L.ts("Host network I/O by interface", [
        ('sum by (interface) (rate(container_network_receive_bytes_total{id="/"}[5m]))', "rx {{interface}}"),
        ('sum by (interface) (rate(container_network_transmit_bytes_total{id="/"}[5m]))', "tx {{interface}}")],
        unit="Bps", w=24, h=8, legend_table=True)

    return dashboard("container-incident", "Container Restart & OOM Watch",
                     ["goat-obs", "cadvisor", "containers"], L, links_to=True,
                     description="Restart/OOM/throttle incident view over cAdvisor.")


# ==========================================================================
# 8. Observability Self-Health  (prometheus meta + up matrix + ALERTS)
# ==========================================================================
def d_meta():
    L = Layout()
    L.text("", (
        "### Observability Self-Health (meta)\n"
        "\"Is my monitoring even working?\" A single-glance scrape-target up/down "
        "matrix, currently-firing alerts (the `ALERTS` series), scrape durations, "
        "and TSDB growth. This is the board that catches the *co-resident-obs* "
        "failure mode where the thing watching dies silently. Grafana strength: "
        "the `ALERTS` table + a `up` state-timeline across every job."), h=4)

    L.stat("Targets up",
           [('sum(up)', "")], w=4, h=4, text_mode="value")
    L.stat("Targets total",
           [('count(up)', "")], w=4, h=4, text_mode="value")
    L.stat("Targets DOWN",
           [('count(up == 0) or vector(0)', "")], w=4, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}])
    L.stat("Alerts firing",
           [('count(ALERTS{alertstate="firing"})', "")], w=4, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 1},
                       {"color": "red", "value": 3}])
    L.stat("TSDB head series",
           [('prometheus_tsdb_head_series', "")], w=4, h=4, desc="active series cardinality")
    L.stat("Rule eval failures/s",
           [('sum(rate(prometheus_rule_evaluation_failures_total[5m]))', "")], w=4, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 0.001}])

    L.timeline("Scrape target up/down (all jobs)",
               [('up', "{{job}} {{instance}}")], h=9,
               desc="every scrape target on one timeline — the monitoring-is-working glance")

    L.table("Currently firing alerts", [
        ('ALERTS{alertstate="firing"}', ""),
    ], w=12, h=8, desc="alertname / severity / component labels",
       transformations=[{"id": "organize", "options": {"excludeByName": {
           "Time": True, "Value": True, "alertstate": True, "__name__": True}}}])
    L.ts("Scrape duration by job",
         [('max by (job) (scrape_duration_seconds)', "{{job}}")], unit="s", w=12, h=8,
         legend_table=True, desc="rising = an exporter getting slow")

    L.ts("Samples scraped by job",
         [('sum by (job) (scrape_samples_scraped)', "{{job}}")], w=12, h=8, legend_table=True)
    L.ts("TSDB head series + appended samples/s", [
        ('prometheus_tsdb_head_series', "head series"),
        ('rate(prometheus_tsdb_head_samples_appended_total[5m])', "samples/s appended")],
        w=12, h=8)

    return dashboard("observability-self-health", "Observability Self-Health",
                     ["goat-obs", "meta", "prometheus"], L, links_to=True,
                     description="Scrape up/down matrix, firing alerts, TSDB growth — is monitoring working?")


# ==========================================================================
# 9. NetBird Mesh Control Plane  (signal + enroll + relay reach)  [DEFENSIVE]
# ==========================================================================
def d_mesh():
    L = Layout()
    L.text("", (
        "### NetBird Mesh Control Plane\n"
        "Enrollment-path health: the signal-server liveness + Go runtime, the "
        "public enrollment endpoint reachability (blackbox HTTPS to `netbird.…`), "
        "and relay-tier reachability (ICMP). **signal-server app-metric names are "
        "not yet referenced in-repo** — this board uses `up` + `go_*`/`process_*` "
        "(which Go exporters always emit) and the blackbox signals, so it lights up "
        "even before app metrics are confirmed."), h=4)

    L.stat("signal-server up", [('up{job="signal-server"}', "")], w=4, h=4, text_mode="value",
           mappings=[{"type": "value", "options": {
               "1": {"text": "UP", "color": "green"}, "0": {"text": "DOWN", "color": "red"}}}])
    L.stat("Enroll endpoint up",
           [('max(probe_success{job="blackbox-https", instance=~".*netbird.*"})', "")],
           w=4, h=4, text_mode="value",
           mappings=[{"type": "value", "options": {
               "1": {"text": "UP", "color": "green"}, "0": {"text": "DOWN", "color": "red"}}}])
    L.stat("Relays reachable (ICMP)",
           [('sum(probe_success{job="blackbox-icmp"})', "")], w=4, h=4, text_mode="value")
    L.stat("signal goroutines",
           [('go_goroutines{job="signal-server"}', "")], w=4, h=4,
           desc="leak detector — steadily climbing is a problem")
    L.stat("signal RSS",
           [('process_resident_memory_bytes{job="signal-server"}', "")], unit="bytes", w=4, h=4)
    L.stat("signal uptime",
           [('time() - process_start_time_seconds{job="signal-server"}', "")], unit="s", w=4, h=4)

    L.timeline("Mesh control-plane liveness", [
        ('up{job="signal-server"}', "signal-server"),
        ('max(probe_success{job="blackbox-https", instance=~".*netbird.*"})', "enroll endpoint"),
    ], h=7)

    L.ts("Relay ICMP latency",
         [('probe_icmp_duration_seconds{job="blackbox-icmp"}', "{{instance}}")],
         unit="s", w=12, h=8, legend_table=True)
    L.ts("Enroll endpoint probe duration",
         [('probe_duration_seconds{job="blackbox-https", instance=~".*netbird.*"}', "{{instance}}")],
         unit="s", w=12, h=8)

    L.ts("signal-server goroutines + threads", [
        ('go_goroutines{job="signal-server"}', "goroutines"),
        ('go_threads{job="signal-server"}', "threads")], w=12, h=8)
    L.ts("signal-server CPU + memory", [
        ('rate(process_cpu_seconds_total{job="signal-server"}[5m])', "cpu cores"),
        ('process_resident_memory_bytes{job="signal-server"} / 1e6', "RSS MB")], w=12, h=8)

    return dashboard("mesh-control-plane", "NetBird Mesh Control Plane",
                     ["goat-obs", "netbird", "mesh"], L, links_to=True,
                     description="Signal-server + enrollment-endpoint + relay reachability (signal app metrics defensive).")


# ==========================================================================
# 10. Capacity & Forecast — 7d trends with predict_linear  (cross-domain)
# ==========================================================================
def d_capacity():
    L = Layout()
    L.text("", (
        "### Capacity & Forecast (7-day trends)\n"
        "Where are we **heading**? `predict_linear` runway on the resources that "
        "actually fill up — root + NVMe disk, memory, the audit bucket, the largest "
        "Postgres DB, TSDB cardinality. Grafana strength: forecasting math turned "
        "into honest \"days until\" stats. Open this board on a `now-7d` window."), h=4)

    L.stat("Days to / full",
           [('node_filesystem_avail_bytes{mountpoint="/"} / '
             'clamp_min(-deriv(node_filesystem_avail_bytes{mountpoint="/"}[24h]),1) / 86400', "")],
           unit="d", w=6, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 14},
                       {"color": "green", "value": 60}])
    L.stat("Days to NVMe full",
           [('node_filesystem_avail_bytes{mountpoint="$mount"} / '
             'clamp_min(-deriv(node_filesystem_avail_bytes{mountpoint="$mount"}[24h]),1) / 86400', "")],
           unit="d", w=6, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 14},
                       {"color": "green", "value": 60}])
    L.stat("Days to audit-bucket full",
           [('(minio_bucket_quota_total_bytes{bucket="audit-integrity"} - '
             'minio_bucket_usage_total_bytes{bucket="audit-integrity"}) / '
             'clamp_min(deriv(minio_bucket_usage_total_bytes{bucket="audit-integrity"}[24h]),1) / 86400', "")],
           unit="d", w=6, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 7},
                       {"color": "green", "value": 30}])
    L.stat("TSDB series (7d slope)",
           [('prometheus_tsdb_head_series', "")], w=6, h=4, graph=True,
           desc="cardinality creep = the silent obs cost")

    L.ts("Disk free trend + 7d projection", [
        ('node_filesystem_avail_bytes{mountpoint="/"}', "/ free"),
        ('predict_linear(node_filesystem_avail_bytes{mountpoint="/"}[24h], 7*86400)', "/ projected +7d"),
        ('node_filesystem_avail_bytes{mountpoint="$mount"}', "nvme free"),
        ('predict_linear(node_filesystem_avail_bytes{mountpoint="$mount"}[24h], 7*86400)', "nvme projected +7d"),
    ], unit="bytes", w=12, h=8, legend_table=True)
    L.ts("Memory available trend + projection", [
        ('node_memory_MemAvailable_bytes', "available"),
        ('predict_linear(node_memory_MemAvailable_bytes[24h], 7*86400)', "projected +7d")],
        unit="bytes", w=12, h=8)

    L.ts("Audit bucket usage + projection", [
        ('minio_bucket_usage_total_bytes{bucket="audit-integrity"}', "usage"),
        ('minio_bucket_quota_total_bytes{bucket="audit-integrity"}', "quota"),
        ('predict_linear(minio_bucket_usage_total_bytes{bucket="audit-integrity"}[24h], 7*86400)', "projected +7d")],
        unit="bytes", w=12, h=8)
    L.ts("Postgres DB sizes trend",
         [('pg_database_size_bytes', "{{datname}}")], unit="bytes", w=12, h=8, legend_table=True)

    return dashboard("capacity-forecast", "Capacity & Forecast (7d)",
                     ["goat-obs", "capacity", "forecast"], L, links_to=True,
                     time_from="now-7d", refresh="5m",
                     description="predict_linear runway across disk/mem/audit/postgres/TSDB.", templating=[MOUNT_VAR])


# ==========================================================================
# 11. Incident Home — curated single-pane portal with drill-down links
# ==========================================================================
def d_home():
    L = Layout()
    L.text("", (
        "# goat-obs — Operations Home\n"
        "One glance across every domain, then drill via the **Dashboards** dropdown "
        "(top-right) or the links below. Each stat is the headline of a detailed "
        "board. This proves the Grafana-as-portal pattern: a home that ties the "
        "suite together rather than 11 disconnected tabs."), h=4)

    # headline stats, one per domain
    L.stat("Targets DOWN", [('count(up == 0) or vector(0)', "")], w=3, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}])
    L.stat("Alerts firing", [('count(ALERTS{alertstate="firing"})', "")], w=3, h=4,
           text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 1}])
    L.stat("Endpoints up",
           [('sum(probe_success{job="blackbox-https"})', "")], w=3, h=4, text_mode="value")
    L.stat("Soonest cert expiry",
           [('min(probe_ssl_earliest_cert_expiry{job="blackbox-https"} - time())', "")],
           unit="s", w=3, h=4,
           thresholds=[{"color": "red", "value": None}, {"color": "yellow", "value": 604800},
                       {"color": "green", "value": 2592000}])
    L.stat("CPU used",
           [('100 - (avg(rate(node_cpu_seconds_total{mode="idle"}[5m])) * 100)', "")],
           unit="percent", w=3, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 80},
                       {"color": "red", "value": 90}])
    L.stat("Mem used",
           [('(1 - node_memory_MemAvailable_bytes / node_memory_MemTotal_bytes) * 100', "")],
           unit="percent", w=3, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 80},
                       {"color": "red", "value": 90}])
    L.stat("/ disk used",
           [('(1 - node_filesystem_avail_bytes{mountpoint="/"} / '
             'node_filesystem_size_bytes{mountpoint="/"}) * 100', "")],
           unit="percent", w=3, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 80},
                       {"color": "red", "value": 85}])
    L.stat("Audit quota used",
           [('100 * minio_bucket_usage_total_bytes{bucket="audit-integrity"} / on(bucket) group_left() minio_bucket_quota_total_bytes{bucket="audit-integrity"}', "")],
           unit="percent", w=3, h=4,
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 85},
                       {"color": "red", "value": 95}])
    L.stat("Fabric samples/s",
           [('sum(rate(goat_audit_samples_total[5m]))', "")], unit="ops", w=3, h=4, graph=True)
    L.stat("SILENT vendors",
           [('count(goat_expected_vendor{active="true"} == 1 unless on(vendor) '
             '(sum by (vendor) (increase(goat_audit_samples_total[15m])) > 0))', "")],
           w=3, h=4, text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "red", "value": 1}])
    L.stat("Container restarts (24h)",
           [('sum(changes(container_start_time_seconds[24h]) * on (id) group_left(name) goat_container_info)', "")], w=3, h=4,
           text_mode="value",
           thresholds=[{"color": "green", "value": None}, {"color": "yellow", "value": 1}])
    L.stat("Relays reachable",
           [('sum(probe_success{job="blackbox-icmp"})', "")], w=3, h=4, text_mode="value")

    L.timeline("All scrape targets", [('up', "{{job}}")], h=6)
    L.table("Firing alerts", [('ALERTS{alertstate="firing"}', "")], w=24, h=6,
            transformations=[{"id": "organize", "options": {"excludeByName": {
                "Time": True, "Value": True, "alertstate": True, "__name__": True}}}])

    links_md = "### Detailed boards\n" + "\n".join(
        f"- **{t}** → `/d/{u}`" for u, t in [
            ("relay-fleet-health", "Relay Fleet Health"),
            ("goatnet-service-slo", "Service SLO & Uptime"),
            ("edge-http-red", "Edge HTTP — RED + Latency"),
            ("postgres-health", "Postgres"),
            ("fabric-tenants", "Fabric Tenants & Silent-Denial"),
            ("audit-storage-integrity", "Audit Integrity & MinIO"),
            ("container-incident", "Container Restart & OOM"),
            ("observability-self-health", "Observability Self-Health"),
            ("mesh-control-plane", "NetBird Mesh Control Plane"),
            ("capacity-forecast", "Capacity & Forecast (7d)"),
        ])
    L.text("", links_md, h=7)

    d = dashboard("incident-home", "Operations Home", ["goat-obs", "home"], L,
                  links_to=True,
                  description="Curated cross-domain home / portal for the goat-obs suite.")
    return d


DASHBOARDS = [
    d_relay, d_slo, d_edge, d_postgres, d_fabric_tenants, d_storage,
    d_containers, d_meta, d_mesh, d_capacity, d_home,
]


def main():
    written = []
    for fn in DASHBOARDS:
        d = fn()
        path = os.path.join(HERE, d["uid"] + ".json")
        with open(path, "w") as f:
            json.dump(d, f, indent=2)
            f.write("\n")
        written.append((d["uid"], d["title"], len(d["panels"])))
    print(f"wrote {len(written)} dashboards:")
    for uid, title, n in written:
        print(f"  {uid:30s} {n:2d} panels  {title}")


if __name__ == "__main__":
    main()
