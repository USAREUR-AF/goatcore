# shellcheck shell=bash
# Two Spock sites in Docker. Sourced by the partition tests in this directory
# (partition-revoke, partition-harness); not run on its own.
#
# Caller sets, before sourcing:
#   IMAGE  postgres-spock image (built from ops/postgres/spock/Dockerfile)
#   PFX    container prefix; the sites are $PFX-a and $PFX-b
#   NET    docker network name
#   PW     postgres + replicator password (test fixture only)
# Optional:
#   TS_PORT_a / TS_PORT_b  publish the site's 5432 on 127.0.0.1:<port>
#   TS_PORT_WAIT           seconds to wait for a held host port (default 120)
#   TS_PORT_STEP           seconds between tries while it is held (default 5)
#
# Substrate: Linux Docker only (CI). Two bidirectional Spock nodes on macOS
# Docker Desktop wedge the daemon.

q() { # q <site a|b> <db> <sql>   (no -i: SQL comes from -c, and -i would drain a caller's while-read input)
  docker exec "$PFX-$1" psql -U postgres -d "$2" -v ON_ERROR_STOP=1 -tAXq -c "$3"
}

# A host port can be held with nothing listening on it. The ports these tests
# publish lie in Linux's ephemeral range (net.ipv4.ip_local_port_range,
# 32768-60999 by default), so the kernel can give one to any loopback client
# connection as its source port: a psql, a curl health probe, another run.
# While that connection is open, and for the 60 s it then spends in TIME-WAIT,
# docker cannot bind the port, and the port frees by itself.

ts_port_holders() { # ts_port_holders <port>  -> "<state> <local> <peer>" per socket that blocks 127.0.0.1:<port>
  command -v ss >/dev/null 2>&1 || return 0
  ss -Htan "sport = :$1" 2>/dev/null | awk -v p="$1" '
    { addr = $4; sub(/%[^:]*:/, ":", addr) }
    addr ~ ("^(127\\.0\\.0\\.1|0\\.0\\.0\\.0|\\*|\\[::\\]|\\[::ffff:127\\.0\\.0\\.1\\]):" p "$") { print $1, $4, $5 }'
}

# Run `docker run ...` that publishes <port> on 127.0.0.1. When the port is
# held by a client connection (open or in TIME-WAIT), remove the container
# docker half-made, say what holds the port, and try again every TS_PORT_STEP
# seconds for up to TS_PORT_WAIT seconds. A listener does not free by itself,
# so that stops at once, naming the container that publishes the port; so
# does docker's "port is already allocated", which is another container of
# this docker publishing it. Any other failure is docker's own, passed through
# with its exit status.
ts_run_published() { # ts_run_published <port> docker run <args...>
  local port="$1"; shift
  local name="" prev="" a
  for a in "$@"; do
    [[ "$prev" == --name ]] && name="$a"
    prev="$a"
  done
  local wait="${TS_PORT_WAIT:-120}" step="${TS_PORT_STEP:-5}" waited=0 rc holders owner range err
  range="$(tr -s '[:space:]' '-' < /proc/sys/net/ipv4/ip_local_port_range 2>/dev/null | sed 's/-$//')"
  err="$(mktemp)"
  while :; do
    rc=0
    "$@" 2>"$err" || rc=$?
    if [[ "$rc" == 0 ]]; then rm -f "$err"; return 0; fi
    if grep -q 'port is already allocated' "$err"; then
      [[ -n "$name" ]] && docker rm -f "$name" >/dev/null 2>&1
      owner="$(docker ps --filter "publish=$port" --format '{{.Names}}' 2>/dev/null | paste -sd, -)"
      echo "::error::127.0.0.1:$port is published by ${owner:-another container of this docker}, and will not free by itself" >&2
      rm -f "$err"; return "$rc"
    fi
    if ! grep -q 'address already in use' "$err"; then
      cat "$err" >&2; rm -f "$err"; return "$rc"
    fi
    [[ -n "$name" ]] && docker rm -f "$name" >/dev/null 2>&1
    holders="$(ts_port_holders "$port")"
    if grep -q '^LISTEN' <<<"$holders"; then
      owner="$(docker ps --filter "publish=$port" --format '{{.Names}}' 2>/dev/null | paste -sd, -)"
      echo "::error::127.0.0.1:$port has a listener, ${owner:-not a container this docker runs}, and will not free by itself: $(tr '\n' ' ' <<<"$holders")" >&2
      rm -f "$err"; return "$rc"
    fi
    if [[ "$waited" -ge "$wait" ]]; then
      echo "::error::127.0.0.1:$port is still held after ${waited}s, by ${holders:-no socket ss can see}" >&2
      cat "$err" >&2; rm -f "$err"; return "$rc"
    fi
    echo "  127.0.0.1:$port is held by ${holders:-no socket ss can see}: a client connection's source port, since the port is in the kernel's ephemeral range${range:+ ($range)}; it frees by itself (TIME-WAIT lasts 60s). Trying again in ${step}s (${waited}s of ${wait}s)" >&2
    sleep "$step"
    waited=$((waited + step))
  done
}

ts_wait_ready() { # ts_wait_ready <container>
  for _ in $(seq 1 90); do
    docker exec "$1" pg_isready -h 127.0.0.1 -p 5432 -U postgres >/dev/null 2>&1 && return 0
    sleep 2
  done
  echo "::error::$1 never accepted a TCP connection"; docker logs "$1" 2>&1 | tail -40; return 1
}

# Start both sites, check the ADR 1193 apply settings baked into the image,
# and create the replicator role the subscriptions connect as.
ts_up() {
  docker network create "$NET" >/dev/null
  local s port
  for s in a b; do
    port="TS_PORT_$s"
    if [[ -n "${!port:-}" ]]; then
      ts_run_published "${!port}" docker run -d --name "$PFX-$s" --network "$NET" -e POSTGRES_PASSWORD="$PW" \
        -p "127.0.0.1:${!port}:5432" "$IMAGE" \
        postgres -c config_file=/etc/postgresql/postgresql.conf -c cluster_name="site_$s" >/dev/null
    else
      docker run -d --name "$PFX-$s" --network "$NET" -e POSTGRES_PASSWORD="$PW" "$IMAGE" \
        postgres -c config_file=/etc/postgresql/postgresql.conf -c cluster_name="site_$s" >/dev/null
    fi
  done
  for s in a b; do ts_wait_ready "$PFX-$s"; done

  local got
  for s in a b; do
    got="$(q "$s" postgres "SELECT current_setting('spock.exception_behaviour') || ' ' || current_setting('spock.save_resolutions');")"
    [[ "$got" == "discard on" ]] || { echo "::error::$PFX-$s spock.exception_behaviour/save_resolutions = '$got', want 'discard on'"; return 1; }
  done
  echo "  exception_behaviour=discard save_resolutions=on"

  for s in a b; do
    q "$s" postgres "CREATE ROLE replicator WITH LOGIN SUPERUSER PASSWORD '$PW';"
    docker exec --user root "$PFX-$s" bash -c \
      "echo 'host all replicator 0.0.0.0/0 scram-sha-256' >> /etc/postgresql/pg_hba.conf"
    q "$s" postgres "SELECT pg_reload_conf();" >/dev/null
  done
}

ts_down() { # ts_down [extra containers...]
  docker rm -f "$PFX-a" "$PFX-b" "$@" >/dev/null 2>&1 || true
  docker network rm "$NET" >/dev/null 2>&1 || true
}

ts_node_create() { # ts_node_create <db>
  local s
  for s in a b; do
    q "$s" "$1" "SELECT spock.node_create('site_$s', 'host=$PFX-$s dbname=$1 user=replicator password=$PW');" >/dev/null
  done
}

# Subscriptions both ways, as spock-mesh-peering and ory-narrow-repset create
# them: no initial sync, no forwarded origins.
ts_sub_create() { # ts_sub_create <db> <repset> [repset...]
  local db="$1"; shift
  local sets; sets="ARRAY['$(IFS=,; echo "$*" | sed "s/,/','/g")']"
  local sub prov
  for pair in "a b" "b a"; do
    read -r sub prov <<<"$pair"
    q "$sub" "$db" "SELECT spock.sub_create(
        subscription_name := 'sub_${prov}_to_${sub}',
        provider_dsn      := 'host=$PFX-$prov dbname=$db user=replicator password=$PW',
        replication_sets  := $sets,
        synchronize_data  := false,
        forward_origins   := ARRAY[]::text[]);" >/dev/null
  done
}

ts_wait_replicating() { # ts_wait_replicating <db>...
  local db sub prov st
  for db in "$@"; do
    for pair in "a b" "b a"; do
      read -r sub prov <<<"$pair"
      st=""
      for _ in $(seq 1 60); do
        st=$(q "$sub" "$db" "SELECT status FROM spock.sub_show_status() WHERE subscription_name='sub_${prov}_to_${sub}';" | tr -d '[:space:]')
        [[ "$st" == replicating ]] && break
        sleep 1
      done
      [[ "$st" == replicating ]] || { echo "::error::$db sub_${prov}_to_${sub} is '$st'"; return 1; }
    done
  done
}

# Partition = both subscriptions disabled; heal = both enabled.
ts_cut() { # ts_cut <db>...
  local db
  for db in "$@"; do
    q a "$db" "SELECT spock.sub_disable('sub_b_to_a', immediate := true);" >/dev/null
    q b "$db" "SELECT spock.sub_disable('sub_a_to_b', immediate := true);" >/dev/null
  done
}
ts_heal() { # ts_heal <db>...
  local db
  for db in "$@"; do
    q a "$db" "SELECT spock.sub_enable('sub_b_to_a', immediate := true);" >/dev/null
    q b "$db" "SELECT spock.sub_enable('sub_a_to_b', immediate := true);" >/dev/null
  done
}
