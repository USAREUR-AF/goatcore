#!/bin/bash
# Second pass on the relay bench (goatcore #949): relay CPU per cell, and the Go relay
# from NetBird main (983b8258a, includes #8679). Client: stock NetBird. Peers forced to relay.
# Output: $OUT/<rep>-<relay>-<tls>-<transport>-P<n>.json (iperf3 -J from peer-a)
#         $OUT/<rep>-<relay>-<tls>-<transport>.csv  (sampler on the relay process)
set -uo pipefail
S=${BENCH_DIR:?set BENCH_DIR to a directory with ssh_config for hosts ctl, relay, peer-a, peer-b}
OUT=$S/pass2; mkdir -p $OUT
X() { local h=$1; shift; ssh -F $S/ssh_config "$h" "$@"; }
RELAY_IP=${RELAY_IP:?relay private address}; PORT=33080; CTL=${CTL_IP:?control private address}
log() { echo "$(date -u +%H:%M:%S) $*" | tee -a $OUT/drive.log; }

log "control: fresh account"
KEY=$(X ctl "sudo /opt/rcmp/control-ctl.sh start $CTL 8080 rels://$RELAY_IP:$PORT" | tail -1)
[ -n "$KEY" ] || { log "no setup key"; exit 1; }
X peer-b 'pkill -x iperf3; nohup iperf3 -s >/dev/null 2>&1 </dev/null &' ; true

CELLS="goat:ktls:ws goat:user:ws goat:ktls:quic go:user:ws go:user:quic gomain:user:ws gomain:user:quic"
for rep in 1 2; do
  for c in $CELLS; do
    IFS=: read relay tls tr <<<"$c"; tag="$rep-$relay-$tls-$tr"
    log "cell $tag"
    X peer-a 'sudo /opt/rcmp/peer-ctl.sh down' ; X peer-b 'sudo /opt/rcmp/peer-ctl.sh down'
    r=$(X relay "sudo /opt/rcmp/relay-ctl.sh start $relay $tls $RELAY_IP $PORT $RELAY_IP:$PORT") || { log "relay start failed: $r"; continue; }
    echo "$r" > $OUT/$tag.relay.txt
    rpid=$(sed -n 's/^pid=\([0-9]*\).*/\1/p' <<<"$r")
    X relay "sudo nohup python3 /opt/rcmp/sampler.py $rpid /opt/rcmp/pass2-$tag.csv 1 >/dev/null 2>&1 </dev/null & echo \$! | sudo tee /opt/rcmp/sampler.pid >/dev/null"
    X peer-b "sudo /opt/rcmp/peer-ctl.sh up stock $tr http://$CTL:8080 $KEY peer-b" >/dev/null
    X peer-a "sudo /opt/rcmp/peer-ctl.sh up stock $tr http://$CTL:8080 $KEY peer-a" >/dev/null
    X peer-b 'pkill -x iperf3; iperf3 -s -D; sleep 0.5; pgrep -x iperf3 >/dev/null' || log "iperf3 server failed"
    ip=""
    for _ in $(seq 1 60); do
      ip=$(X peer-a "sudo /opt/rcmp/bin/netbird status --json" 2>/dev/null | python3 -c '
import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit()
for p in d.get("peers",{}).get("details",[]):
    if p.get("status")=="Connected" and p.get("connectionType")=="Relayed": print(p["netbirdIp"].split("/")[0]); break')
      [ -n "$ip" ] && break; sleep 2
    done
    [ -n "$ip" ] || { log "peers not connected via relay: $tag"; continue; }
    X peer-a "sudo /opt/rcmp/bin/netbird status --json" > $OUT/$tag.status.json
    for P in 1 8; do
      X peer-a "iperf3 -c $ip -t 20 -O 2 -P $P -J" > $OUT/$tag-P$P.json
      log "  P$P $(python3 -c "import json;d=json.load(open('$OUT/$tag-P$P.json'));print(round(d['end']['sum_received']['bits_per_second']/1e6),'Mbit/s')" 2>/dev/null)"
    done
    X relay 'sudo kill $(cat /opt/rcmp/sampler.pid) 2>/dev/null'
    X relay "cat /opt/rcmp/pass2-$tag.csv" > $OUT/$tag.csv
  done
done
X peer-a 'sudo /opt/rcmp/peer-ctl.sh down'; X peer-b 'sudo /opt/rcmp/peer-ctl.sh down; pkill -x iperf3'; X relay 'sudo /opt/rcmp/relay-ctl.sh stop'
log "done"
