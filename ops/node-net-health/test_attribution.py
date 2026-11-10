#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
#
# Tests for the ADR 1128 bandwidth-attribution additions to nnh-snapshot.
#
# The parser is the load-bearing piece and it is parsing a text format from
# another program, so it is tested against REAL-SHAPED `ss -tinpH` output
# rather than a tidied-up invention, because two silent failures live here:
#
#   1. `ss -t` alone emits NO Netid column (ss only adds it when more than one
#      family is requested), so reusing the inventory's SS_LINE regex matched
#      nothing at all and every counter stayed at zero.
#   2. A socket that has moved bytes in only one direction OMITS the other
#      field rather than printing 0.
#
# Run: python3 ops/node-net-health/test_attribution.py

import importlib.util
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("nnh", os.path.join(HERE, "nnh-snapshot.py"))
nnh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(nnh)

FAILURES = []


def check(name, got, want):
    if got != want:
        FAILURES.append(f"{name}\n     got:  {got!r}\n     want: {want!r}")
    else:
        print(f"  ok  {name}")


# Verbatim shape of `ss -tinpH` on Ubuntu 24.04 / iproute2 6.1: socket line,
# then an indented info line. Note the LACK of a Netid column.
SS_FIXTURE = """\
ESTAB      0      0            100.127.1.23:7447        100.127.1.24:44120 users:(("zenohd",pid=901,fd=31))
\t cubic wscale:7,7 rto:204 rtt:0.31/0.05 mss:1448 cwnd:10 bytes_sent:98765 bytes_acked:98765 bytes_received:43210 segs_out:88 segs_in:71 send 373Mbps lastsnd:12 rcv_space:14480 minrtt:0.09
ESTAB      0      0            100.127.1.23:53124       100.127.1.20:5432 users:(("postgres",pid=1502,fd=9))
\t cubic wscale:7,7 rto:212 rtt:1.9/0.4 mss:1448 cwnd:10 bytes_sent:500 bytes_acked:500 bytes_received:1200000 segs_out:9 segs_in:840 send 61Mbps lastsnd:4 rcv_space:14480 minrtt:1.4
ESTAB      0      0            192.168.1.172:41022        198.51.100.48:443 users:(("dockerd",pid=770,fd=22))
\t cubic wscale:8,7 rto:236 rtt:36/2 mss:1400 cwnd:32 bytes_sent:8100 bytes_received:734003200 segs_out:120 segs_in:512000 send 9.9Mbps lastsnd:8 rcv_space:14600 minrtt:33
LISTEN     0      4096         0.0.0.0:9100               0.0.0.0:* users:(("node_exporter",pid=655,fd=3))
\t cubic cwnd:10
ESTAB      0      0            100.127.1.23:38812       100.127.1.22:443 users:(("chronyd",pid=402,fd=6))
\t cubic wscale:7,7 rto:204 rtt:0.4/0.1 mss:1448 cwnd:10 bytes_sent:64 segs_out:2 segs_in:1 send 28Mbps lastsnd:900 rcv_space:14480 minrtt:0.3
"""


def test_parser():
    c = nnh.tcp_byte_counters(raw=SS_FIXTURE)
    check("parses every ESTAB socket, skips LISTEN", len(c), 4)
    check("zenoh fabric socket counters",
          c[("host", "100.127.1.23:7447", "100.127.1.24:44120")],
          {"sent": 98765, "recv": 43210})
    check("replication socket — receive-heavy",
          c[("host", "100.127.1.23:53124", "100.127.1.20:5432")],
          {"sent": 500, "recv": 1200000})
    check("image pull off-mesh — 700MB in",
          c[("host", "192.168.1.172:41022", "198.51.100.48:443")],
          {"sent": 8100, "recv": 734003200})
    # The bug that would have silently mis-read: bytes_received absent entirely.
    check("one-direction socket: missing field reads 0, not a parse failure",
          c[("host", "100.127.1.23:38812", "100.127.1.22:443")],
          {"sent": 64, "recv": 0})
    check("LISTEN socket is absent",
          ("host", "0.0.0.0:9100", "0.0.0.0:*") in c, False)


def test_counter_accumulation():
    ctr = {}
    nnh.bump(ctr, "workload", ["fabric", "rx"], 100)
    nnh.bump(ctr, "workload", ["fabric", "rx"], 50)
    check("deltas accumulate into a running total", ctr["workload|fabric|rx"], 150)
    nnh.bump(ctr, "workload", ["fabric", "rx"], 0)
    nnh.bump(ctr, "workload", ["fabric", "rx"], -900)
    check("zero and negative deltas are refused (counter-reset guard)",
          ctr["workload|fabric|rx"], 150)


def test_categorize():
    rules = [
        {"name": "ghcr-pulls", "comm": ["dockerd"], "remote_ports": [443],
         "category": "image-pull"},
        {"name": "apt-mirrors", "comm": ["apt"], "remote_ports": [80, 443]},
        {"name": "spock-replication", "remote_ports": [5432],
         "category": "replication"},
    ]

    def conn(bucket, rule="", comm="", proto="tcp", remote="1.2.3.4:443"):
        return {"bucket": bucket, "baseline_rule": rule, "comm": comm,
                "container": "", "proto": proto,
                "local_addr": "100.127.1.23:5000", "remote_addr": remote}

    check("fabric bucket is structural",
          nnh.categorize(conn("fabric"), rules), "fabric")
    check("undeclared bucket mirrors the security bucket",
          nnh.categorize(conn("undeclared"), rules), "undeclared")
    check("baseline rule carries its category",
          nnh.categorize(conn("baseline", "ghcr-pulls", "dockerd"), rules), "image-pull")
    check("declared rule with NO category is visible, not silently bucketed",
          nnh.categorize(conn("baseline", "apt-mirrors", "apt"), rules), "other-declared")
    # The case that motivates consulting rules for mesh traffic: a Postgres
    # session to a PEER is replication, not an undifferentiated "mesh" blob.
    check("in-tunnel Postgres is replication, not generic mesh",
          nnh.categorize(conn("mesh", remote="100.127.1.20:5432"), rules),
          "replication")
    check("in-tunnel traffic matching no rule is other-declared",
          nnh.categorize(conn("mesh", remote="100.127.1.20:9999"), rules),
          "other-declared")

    # The second double-count. A connection ACCEPTED on :443 is web traffic the
    # access log already breaks out; counting it here too would inflate the
    # workload plane by the whole of the web tier.
    def accepted(local, remote="100.127.1.24:51234"):
        return {"bucket": "mesh", "baseline_rule": "", "comm": "traefik",
                "container": "traefik", "proto": "tcp",
                "local_addr": local, "remote_addr": remote}

    check("socket accepted on :443 is delegated to the HTTP plane",
          nnh.categorize(accepted("100.127.1.23:443"), rules), "http-delegated")
    check("socket accepted on :80 is delegated too",
          nnh.categorize(accepted("100.127.1.23:80"), rules), "http-delegated")
    # The distinguishing case: an OUTBOUND pull to a remote :443 has an
    # ephemeral local port and must stay image-pull, not get swallowed as web.
    # Fabric must be decided on PORT/COMM, not on the security bucket.
    # classify() tests mesh BEFORE fabric, so a zenoh session to a mesh peer
    # arrives here as bucket="mesh"; a `bucket == "fabric"` test alone reported
    # ZERO fabric bytes.
    check("zenoh to a mesh peer is fabric despite bucket=mesh",
          nnh.categorize(conn("mesh", remote="100.127.0.217:7447"), rules), "fabric")
    check("zenohd by comm is fabric even on an odd port",
          nnh.categorize(conn("mesh", comm="zenohd", remote="100.127.0.217:9999"), rules),
          "fabric")

    # The transport underlay. These carry the mesh; the transport plane already
    # counts them. Counting them here too double-counts the entire mesh.
    check("rathole relay tunnel is transport-underlay, not workload",
          nnh.categorize(conn("baseline", "rathole-relays", "rathole",
                              remote="203.0.113.205:2333"), rules),
          "transport-underlay")
    check("netbird control connection is transport-underlay",
          nnh.categorize(conn("mesh", comm="netbird", remote="203.0.113.205:443"), rules),
          "transport-underlay")

    check("outbound to a remote :443 is NOT delegated (local port is ephemeral)",
          nnh.categorize(conn("baseline", "ghcr-pulls", "dockerd",
                              remote="198.51.100.48:443"), rules),
          "image-pull")


def test_textfile_render():
    with tempfile.TemporaryDirectory() as d:
        nnh.TEXTFILE = os.path.join(d, "goat_net_attribution.prom")
        ctr = {
            "transport|mesh-relayed|rx": 4096,
            # Workload counters carry the netns they were read in; a
            # two-part key is the pre-label shape and renders netns="unknown".
            "workload|fabric|tx|host": 2048,
            "peer|orin-a.zagros|mesh-p2p|rx": 512,
            "workload_peer|replication|100.127.1.20|rx": 64,
        }
        nnh.write_textfile(ctr, 60, 12, 30, True, [
            ("goat_net_peer_latency_ms", {"peer": "a.zagros", "class": "mesh-p2p"}, 4.4),
            ("goat_net_peer_latency_measured", {"peer": "a.zagros", "class": "mesh-p2p"}, 1),
            # netbird reports 0 for peers it has not timed. 0 ms must never
            # render as an instant link.
            ("goat_net_peer_latency_ms", {"peer": "b.zagros", "class": "mesh-relayed"}, 0),
            ("goat_net_peer_latency_measured", {"peer": "b.zagros", "class": "mesh-relayed"}, 0),
        ])
        with open(nnh.TEXTFILE) as source_file:
            body = source_file.read()
    # No host-identifying label: the scrape config supplies `host`, and a
    # colliding textfile label would be renamed exported_host by Prometheus,
    # silently breaking every `by (host)` join in the recording rules.
    check("textfile emits NO host label of its own", "node=" in body, False)
    for want in [
        'goat_net_transport_bytes_total{class="mesh-relayed",direction="rx"} 4096',
        'goat_net_workload_bytes_total{category="fabric",direction="tx",netns="host"} 2048',
        'goat_net_peer_bytes_total{peer="orin-a.zagros",class="mesh-p2p",direction="rx"} 512',
        'goat_net_workload_peer_bytes_total{category="replication",peer="100.127.1.20",direction="rx"} 64',
        'goat_net_attribution_mesh_plane_up 1',
        'goat_net_attribution_sample_gap_seconds 60',
        'goat_net_peer_latency_ms{peer="a.zagros",class="mesh-p2p"} 4.4',
        'goat_net_peer_latency_measured{peer="a.zagros",class="mesh-p2p"} 1',
        'goat_net_peer_latency_measured{peer="b.zagros",class="mesh-relayed"} 0',
    ]:
        check(f"textfile emits: {want.split('{')[0]}", want in body, True)
    # 4 byte families + 7 honesty gauges (gap, up, mesh_plane_up,
    # peers_sampled, sockets_sampled, and the F-331 pair netns_targets /
    # netns_read, which say how much of the node was actually enumerated)
    # + 2 latency families, emitted because the fixture above passes samples.
    check("every family declares a TYPE", body.count("# TYPE"), 13)
    for want in ("goat_net_attribution_netns_targets",
                 "goat_net_attribution_netns_read"):
        check(f"textfile emits: {want}", want in body, True)
    check("every TYPE has a HELP", body.count("# HELP"), body.count("# TYPE"))


def test_textfile_missing_dir_is_not_fatal():
    nnh.TEXTFILE = "/proc/definitely-not-writable/x.prom"
    try:
        nnh.write_textfile({}, 0, 0, 0, False, [])
        check("unwritable textfile degrades, does not raise", True, True)
    except Exception as e:
        check("unwritable textfile degrades, does not raise", f"raised {e}", True)





# ---------------------------------------------------------------------------
# F-331 — sockets that live in a container's own network namespace
# ---------------------------------------------------------------------------
#
# The collector ran `ss` once, on the host, so every socket belonging to a
# container with its own netns was structurally invisible: low coverage with
# BOTH catch-all buckets flat zero, the tell that nothing was arriving to be
# categorised, rather than arriving and failing to match a rule.
#
# These test the two things that could silently corrupt data rather than
# merely under-report it.

# `ss -tunapH` shape: Netid State Recv-Q Send-Q Local Peer Process.
HOST_VIEW = """\
tcp   LISTEN 0 4096 172.17.0.5:7447 0.0.0.0:* users:(("docker-proxy",pid=900,fd=4))
tcp   ESTAB  0 0    100.127.1.23:22 100.127.1.24:51000 users:(("sshd",pid=1200,fd=3))
tcp   ESTAB  0 0    172.17.0.1:44000 172.17.0.5:7447 users:(("docker-proxy",pid=900,fd=9))
"""

# The same node, seen from inside the zenoh router's namespace.
ROUTER_VIEW = """\
tcp   LISTEN 0 4096 0.0.0.0:7447 0.0.0.0:* users:(("zenohd",pid=1,fd=31))
tcp   LISTEN 0 4096 172.17.0.5:7448 0.0.0.0:* users:(("zenohd",pid=1,fd=32))
tcp   ESTAB  0 0    172.17.0.5:7447 100.127.1.24:44120 users:(("zenohd",pid=1,fd=44))
"""


def rows_for(view, netns, container=""):
    return nnh.parse_ss_rows(view, {}, {}, netns, netns_container=container)


def test_host_view_still_drops_bridge_plumbing():
    rows = rows_for(HOST_VIEW, "host")
    locals_ = sorted(f'{r["local"][0]}:{r["local"][1]}' for r in rows)
    # The docker-proxy listener on the bridge and the bridge<->bridge flow are
    # plumbing; only the real sshd socket is the node's connection graph. This
    # is unchanged behaviour and must stay unchanged.
    check("host view: bridge listener + bridge<->bridge dropped",
          locals_, ["100.127.1.23:22"])


def test_container_view_keeps_its_listeners():
    rows = rows_for(ROUTER_VIEW, "zenoh-router", container="zenoh-router")
    locals_ = sorted(f'{r["local"][0]}:{r["local"][1]}' for r in rows)
    # THE BUG THIS GUARDS. Seen from the host, a listener bound to 172.17.x is
    # the bridge and is plumbing. Seen from inside the container it is the
    # container's OWN address and the listener is the actual service. Applying
    # the host rule inside the namespace would drop :7448 — one of the ports
    # this whole enumeration exists to find.
    check("container view: keeps both listeners incl. the bridge-bound one",
          locals_, ["0.0.0.0:7447", "172.17.0.5:7447", "172.17.0.5:7448"])
    check("container rows are attributed to the container",
          sorted({r["container"] for r in rows}), ["zenoh-router"])
    check("container rows carry their namespace",
          sorted({r["netns"] for r in rows}), ["zenoh-router"])


def test_host_rows_are_tagged_host():
    rows = rows_for(HOST_VIEW, "host")
    check("host rows carry netns=host", sorted({r["netns"] for r in rows}), ["host"])


# Two containers on the default bridge, both holding 172.17.0.2:443 talking to
# different peers. Before the namespace was part of the key these shared one
# counter entry, so one container's bytes were attributed to the other.
DUP_A = """\
ESTAB 0 0 172.17.0.2:443 100.127.1.20:40000 users:(("traefik",pid=1,fd=7))
\t cubic bytes_sent:1000 bytes_received:2000
"""
DUP_B = """\
ESTAB 0 0 172.17.0.2:443 100.127.1.21:40001 users:(("portal",pid=1,fd=7))
\t cubic bytes_sent:30 bytes_received:40
"""


def test_identical_container_addresses_do_not_collide():
    merged = {}
    merged.update(nnh.tcp_byte_counters(raw=DUP_A, netns="traefik"))
    merged.update(nnh.tcp_byte_counters(raw=DUP_B, netns="portal"))
    check("two containers sharing a bridge address keep separate counters",
          len(merged), 2)
    check("traefik's bytes stay traefik's",
          merged[("traefik", "172.17.0.2:443", "100.127.1.20:40000")],
          {"sent": 1000, "recv": 2000})
    check("portal's bytes stay portal's",
          merged[("portal", "172.17.0.2:443", "100.127.1.21:40001")],
          {"sent": 30, "recv": 40})


def test_same_socket_in_two_namespaces_is_not_summed():
    # The dangerous direction. Under-reporting shows up as low coverage;
    # over-reporting is invisible and inflates the number used to judge
    # headroom, which is why host-networked containers are excluded from
    # enumeration rather than merged.
    a = nnh.tcp_byte_counters(raw=DUP_A, netns="traefik")
    b = nnh.tcp_byte_counters(raw=DUP_A, netns="traefik")
    merged = {}
    merged.update(a)
    merged.update(b)
    check("the same socket seen twice does not double",
          merged[("traefik", "172.17.0.2:443", "100.127.1.20:40000")],
          {"sent": 1000, "recv": 2000})


def test_unreadable_namespace_is_not_silently_clean():
    # run_ss returns "" on any failure; an empty parse must yield no rows
    # rather than raising, and the caller counts it as unread.
    check("empty ss output parses to zero rows", rows_for("", "broken"), [])


for fn in (test_parser, test_counter_accumulation, test_categorize,
           test_textfile_render, test_textfile_missing_dir_is_not_fatal,
           # F-331 — container network namespaces
           test_host_view_still_drops_bridge_plumbing,
           test_container_view_keeps_its_listeners,
           test_host_rows_are_tagged_host,
           test_identical_container_addresses_do_not_collide,
           test_same_socket_in_two_namespaces_is_not_summed,
           test_unreadable_namespace_is_not_silently_clean):
    print(f"\n{fn.__name__}:")
    fn()

print()
if FAILURES:
    print(f"FAIL — {len(FAILURES)} check(s):")
    for f in FAILURES:
        print(f"  x  {f}")
    sys.exit(1)
print("PASS — all attribution checks green")
