#!/usr/bin/env python3
"""Checks a goat-logship render (tests/render.yml) for GHSA-x44g.

check_render.py <dir> <case>; case is one of
  default      every container-stdout source on, mesh-bound
  no-docker    all three container-stdout sources off
  bind-all     logship_bind_all: true
Prints one line per check; exits 1 on any failure.
"""
import sys

import yaml

d, case = sys.argv[1], sys.argv[2]
fails = 0


def check(ok, what):
    global fails
    print(("  ok   " if ok else "  FAIL ") + what)
    fails += 0 if ok else 1


with open(f"{d}/docker-compose.yml") as compose_file:
    compose = yaml.safe_load(compose_file)
with open(f"{d}/vector.yaml") as vector_file:
    vector = yaml.safe_load(vector_file)
svc = compose["services"]
vec = svc["goat-logship"]

# 1. Vector never holds the Docker socket, in any case.
vols = [v.split(":")[0] for v in vec.get("volumes", [])]
check(not any("docker.sock" in v for v in vec.get("volumes", [])),
      "Vector mounts no Docker socket")
check(not any(v.startswith("/var/lib/docker") for v in vols),
      "Vector mounts nothing under /var/lib/docker")

# 2. Vector runs as a non-root user, without capabilities.
user = str(vec.get("user", ""))
uid, _, gid = user.partition(":")
check(uid.isdigit() and int(uid) != 0 and gid.isdigit() and int(gid) != 0,
      f"Vector runs as a non-root uid:gid ({user!r})")
check("0" not in [str(g) for g in vec.get("group_add", [])],
      f"no root group added ({vec.get('group_add')})")
check(vec.get("cap_drop") == ["ALL"], "Vector drops every capability")
check("no-new-privileges:true" in vec.get("security_opt", []),
      "Vector has no-new-privileges")

# 3. The metrics port.
ports = [str(p) for p in vec.get("ports", [])]
if case == "bind-all":
    check(ports == ["9598:9598"], f"bind_all publishes on every interface ({ports})")
else:
    check(ports == ["127.0.0.1:9598:9598", "100.64.0.7:9598:9598", "172.17.0.1:9598:9598"],
          f"metrics port bound to loopback, mesh and bridge gateway only ({ports})")
    check(all(p.count(":") == 2 for p in ports), "no port is published without a host address")

# 4. The Docker API proxy.
proxy = svc.get("goat-logship-docker-proxy")
docker_srcs = {k: v for k, v in vector["sources"].items() if v.get("type") == "docker_logs"}
if case == "no-docker":
    check(proxy is None, "no Docker API proxy when no source reads container stdout")
    check(not docker_srcs, "no docker_logs source rendered")
    check("goat-logship-docker" not in compose.get("networks", {}), "no proxy network")
    check(all("docker.sock" not in str(s.get("volumes", [])) for s in svc.values()),
          "no service mounts the Docker socket")
else:
    sockdir = "/opt/goat-logship/docker-proxy-socket"
    check(proxy is not None, "Docker API proxy service rendered")
    check("/var/run/docker.sock:/var/run/docker.sock" in proxy.get("volumes", []),
          "only the proxy mounts the Docker socket")
    check(str(proxy.get("user")) == "996:997",
          f"proxy runs as its own uid with the collector's gid ({proxy.get('user')})")
    check([str(g) for g in proxy.get("group_add", [])] == ["983"], "proxy gets the socket's group")
    check(proxy.get("read_only") is True and proxy.get("cap_drop") == ["ALL"],
          "proxy: read-only root, no capabilities")
    check(proxy.get("network_mode") == "none" and "networks" not in proxy and "ports" not in proxy,
          "proxy has no network and publishes no port")
    check("goat-logship-docker" not in compose.get("networks", {}), "no proxy TCP network")
    check(f"{sockdir}:/run/goat-logship-docker-proxy" in proxy.get("volumes", []),
          "proxy writes its socket into the socket dir")
    check(f"{sockdir}:/run/goat-logship-docker-proxy:ro" in vec.get("volumes", []),
          "Vector mounts the socket dir read-only")
    ep = "\n".join(proxy.get("entrypoint", []))
    # $$ is compose's escape for a literal $.
    check('chmod 0660 "$$s"' in ep and "nginx -c /etc/goat-logship-docker-proxy/nginx.conf" in ep,
          "proxy entrypoint starts nginx and makes its socket 0660")
    check(sorted(docker_srcs) == ["admit_audit", "oauth2_proxy", "traefik"],
          f"docker_logs sources ({sorted(docker_srcs)})")
    check(all(s.get("docker_host") == "unix:///run/goat-logship-docker-proxy/api.sock" for s in docker_srcs.values()),
          "every docker_logs source goes through the proxy's unix socket")
    with open(f"{d}/docker-proxy.nginx.conf") as proxy_file:
        conf = proxy_file.read()
    listens = [l.strip() for l in conf.splitlines() if l.strip().startswith("listen")]
    check(listens == ["listen unix:/run/goat-logship-docker-proxy/api.sock;"],
          f"proxy listens on the unix socket only, no TCP ({listens})")
    check("limit_except GET" in conf and "server unix:/var/run/docker.sock;" in conf,
          "proxy config: GET/HEAD only, upstream is the Docker socket")

# 5. The exporter address inside the container and the guard.
check(vector["sinks"]["metrics"]["address"] == "0.0.0.0:9598",
      "exporter listens in its own netns; host exposure is the ports list")
if case not in ("bind-all", "no-counter"):
    with open(f"{d}/logship-expose-guard.nft") as guard_file:
        nft = guard_file.read()
    check("ip daddr 100.64.0.7 tcp dport 9598 counter drop" in nft
          and "ct original ip daddr 100.64.0.7 ct original proto-dst 9598 counter drop" in nft,
          "exposure guard drops non-mesh traffic to the mesh-bound port")
    check('iifname != "lo" iifname != "br-0123456789ab" ip daddr 172.17.0.1 tcp dport 9598 counter drop' in nft
          and 'iifname != "br-0123456789ab" meta l4proto tcp ct original ip daddr 172.17.0.1 ct original proto-dst 9598 counter drop' in nft,
          "exposure guard admits the gateway binding only from the scraper's bridge")

if case == "no-counter":
    nft = open(f"{d}/logship-expose-guard.nft").read()
    check("counter" not in nft.split("table inet goat_logship_expose {", 1)[1]
          and "ip daddr 100.64.0.7 tcp dport 9598 drop" in nft,
          "a kernel without nft counters gets the same drops, uncounted")

sys.exit(1 if fails else 0)
