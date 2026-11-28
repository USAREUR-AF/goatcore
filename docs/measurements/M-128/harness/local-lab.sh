#!/usr/bin/env bash
# A local stand-in for the four cloud machines, to check the harness before
# cloud time is spent. Each machine is an Ubuntu 24.04 container on one docker
# network. Numbers from this lab share one host's cores and are a harness
# check only.
#
#   local-lab.sh up [bridge|host]   bridge: the relay has its own network
#                                   namespace (goat-relay then uses TLS in user
#                                   space, because the kernel TLS module is not
#                                   visible there); host: the relay shares the
#                                   host's namespace and can use kernel TLS.
#   local-lab.sh down
#   local-lab.sh env [bridge|host] [docker|ssh]
#                                   print the hosts.env for the lab. "docker"
#                                   reaches the containers with docker exec;
#                                   "ssh" (bridge only) reaches them over SSH as
#                                   user ubuntu with ~/.ssh/id_ed25519, as on
#                                   the cloud machines.
set -euo pipefail
NET=rcmp-lab
SUBNET=${LAB_SUBNET:-10.98.0.0/24}
PREFIX=${SUBNET%.*}
IMAGE=rcmp-host:24.04

hosts_env() {
  local relay_addr=$PREFIX.20
  if [ "${1:-bridge}" = host ]; then relay_addr=$PREFIX.1; fi
  if [ "${2:-docker}" = ssh ]; then
    printf '%s\n' HOST_MODE=ssh "CONTROL_TARGET=ubuntu@$PREFIX.10" "RELAY_TARGET=ubuntu@$relay_addr" \
      "PEER_A_TARGET=ubuntu@$PREFIX.11" "PEER_B_TARGET=ubuntu@$PREFIX.12"
  else
    printf '%s\n' HOST_MODE=docker CONTROL_TARGET=rcmp-control RELAY_TARGET=rcmp-relay \
      PEER_A_TARGET=rcmp-peer-a PEER_B_TARGET=rcmp-peer-b
  fi
  printf '%s\n' "CONTROL_ADDR=$PREFIX.10" "RELAY_ADDR=$relay_addr" "RELAY_BIND=$relay_addr" \
    "PEER_A_ADDR=$PREFIX.11" "PEER_B_ADDR=$PREFIX.12" RCMP_NO_MODPROBE=1
}

case "${1:-}" in
  env) hosts_env "${2:-bridge}" "${3:-docker}" ;;
  down)
    docker rm -f rcmp-control rcmp-relay rcmp-peer-a rcmp-peer-b >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
    ;;
  up)
    mode=${2:-bridge}
    docker build -q -t "$IMAGE" - >/dev/null <<'DOCKERFILE'
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    iperf3 iproute2 iputils-ping python3 curl ca-certificates kmod procps openssl \
    openssh-server sudo \
 && rm -rf /var/lib/apt/lists/* \
 && (id ubuntu >/dev/null 2>&1 || useradd -m -s /bin/bash ubuntu) \
 && echo "ubuntu ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/ubuntu \
 && mkdir -p /run/sshd /home/ubuntu/.ssh && chown ubuntu:ubuntu /home/ubuntu/.ssh
CMD ["sleep", "infinity"]
DOCKERFILE
    docker network inspect "$NET" >/dev/null 2>&1 || docker network create --subnet "$SUBNET" "$NET" >/dev/null
    docker run -d --init --name rcmp-control --network "$NET" --ip "$PREFIX.10" "$IMAGE" >/dev/null
    if [ "$mode" = host ]; then
      docker run -d --init --name rcmp-relay --network host "$IMAGE" >/dev/null
    else
      docker run -d --init --name rcmp-relay --network "$NET" --ip "$PREFIX.20" "$IMAGE" >/dev/null
    fi
    for p in a b; do
      ip=$PREFIX.11; [ $p = b ] && ip=$PREFIX.12
      docker run -d --init --name "rcmp-peer-$p" --hostname "peer-$p" --network "$NET" --ip "$ip" \
        --privileged --device /dev/net/tun "$IMAGE" >/dev/null
    done
    if [ -f "$HOME/.ssh/id_ed25519.pub" ]; then
      for c in rcmp-control rcmp-relay rcmp-peer-a rcmp-peer-b; do
        docker cp "$HOME/.ssh/id_ed25519.pub" "$c:/home/ubuntu/.ssh/authorized_keys"
        docker exec "$c" bash -c 'chown ubuntu:ubuntu /home/ubuntu/.ssh/authorized_keys; chmod 600 /home/ubuntu/.ssh/authorized_keys'
        # A host-network relay would collide with the host's own sshd.
        if [ "$c" != rcmp-relay ] || [ "$mode" != host ]; then docker exec "$c" /usr/sbin/sshd; fi
      done
    fi
    docker ps --filter name=rcmp- --format '{{.Names}} {{.Status}}'
    ;;
  *) echo "usage: $0 up [bridge|host] | down | env [bridge|host] [docker|ssh]" >&2; exit 2 ;;
esac
