#!/usr/bin/env bash
# Runs as root on the relay machine.
#
#   relay-ctl.sh start <go|goat> <ktls|user> <bind addr> <port> <exposed addr>
#   relay-ctl.sh stop
#   relay-ctl.sh tls-path    prints what the kernel says about the TLS module
#
# The auth secret is read from /opt/rcmp/relay-secret, the certificate and key
# from /opt/rcmp/tls/. One relay runs at a time; start stops any other first.
#
# ktls|user selects whether the kernel TLS module may be used. "user" unloads
# the tls module and blocks it from loading again, so a relay that would hand
# its TLS session to the kernel (goat-relay) falls back to TLS in user space.
# "ktls" loads it. The Go relay uses TLS in user space either way.
set -euo pipefail
action=${1:?action}
D=/opt/rcmp
PIDF=$D/relay.pid

stop_relay() {
  if [ -f "$PIDF" ]; then
    local p; p=$(cat "$PIDF")
    if kill -0 "$p" 2>/dev/null; then
      kill "$p" || true
      for _ in $(seq 1 50); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
      kill -9 "$p" 2>/dev/null || true
    fi
    rm -f "$PIDF"
  fi
  pkill -x goat-relay 2>/dev/null || true
  pkill -x netbird-relay 2>/dev/null || true
}

tls_path() {
  local ulp=absent
  if [ -r /proc/sys/net/ipv4/tcp_available_ulp ]; then
    ulp=$(cat /proc/sys/net/ipv4/tcp_available_ulp)
  fi
  local mod=no
  if [ -d /sys/module/tls ]; then mod=yes; fi
  echo "tls_module_loaded=$mod tcp_available_ulp=[$ulp]"
  if [ -r /proc/net/tls_stat ]; then
    tr -s ' \t' '=' < /proc/net/tls_stat | tr '\n' ' '; echo
  else
    echo "tls_stat=absent"
  fi
}

set_tls_mode() {
  case "$1" in
    ktls)
      rm -f /etc/modprobe.d/rcmp-no-tls.conf
      if [ ! -f $D/no-modprobe ]; then modprobe tls; fi
      ;;
    user)
      if [ ! -f $D/no-modprobe ]; then
        echo "install tls /bin/false" > /etc/modprobe.d/rcmp-no-tls.conf
        if [ -d /sys/module/tls ]; then modprobe -r tls; fi
        if [ -d /sys/module/tls ]; then echo "error: tls module still loaded" >&2; exit 1; fi
      fi
      ;;
    *) echo "unknown tls mode $1" >&2; exit 2 ;;
  esac
}

case "$action" in
  stop) stop_relay ;;
  tls-path) tls_path ;;
  start)
    relay=${2:?relay} mode=${3:?tls mode} bind=${4:?bind} port=${5:?port} exposed=${6:?exposed}
    stop_relay
    set_tls_mode "$mode"
    secret=$(cat $D/relay-secret)
    log=$D/logs/relay-$relay-$mode-$(date -u +%Y%m%dT%H%M%SZ).log
    mkdir -p $D/logs
    case "$relay" in
      goat)
        GOAT_RELAY_AUTH_SECRET=$secret setsid nohup $D/bin/goat-relay \
          --tls-cert $D/tls/relay.crt --tls-key $D/tls/relay.key \
          --bind "$bind:$port" --advertised "$exposed" > "$log" 2>&1 < /dev/null &
        ;;
      go)
        NB_AUTH_SECRET=$secret setsid nohup $D/bin/netbird-relay \
          --listen-address "$bind:$port" --exposed-address "rels://$exposed" \
          --tls-cert-file $D/tls/relay.crt --tls-key-file $D/tls/relay.key \
          --health-listen-address 127.0.0.1:9000 \
          --metrics-port 9091 > "$log" 2>&1 < /dev/null &
        ;;
      *) echo "unknown relay $relay" >&2; exit 2 ;;
    esac
    echo $! > "$PIDF"
    sleep 2
    p=$(cat "$PIDF")
    if ! kill -0 "$p" 2>/dev/null; then
      echo "error: relay exited; log follows" >&2; cat "$log" >&2; exit 1
    fi
    echo "pid=$p log=$log"
    tls_path
    ;;
  *) echo "unknown action $action" >&2; exit 2 ;;
esac
