# simple-bn

A simple boundary node and the boundary monitor. Design and status: [`docs/design/simple-boundary-node.md`](../../docs/design/simple-boundary-node.md); decisions [ADR 1253](../../docs/adr/1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md) and [ADR 1263](../../docs/adr/1263-simple-boundary-node-partner-ipsec-gateway-fails-closed-and-real-data-needs-a-validated-module.md) (stage 2 with IPsec; which stage may carry real data).

| File | What it is |
|---|---|
| `simple_bn_sender.py` | sends notional feeds as ASTERIX CAT034/CAT048 over UDP or TCP (`--transport`) to one receiver, or with `--input relay` forwards a sender's ASTERIX from the mesh unchanged (`--dest none` holds), or with `--input fabric` reads released keys from the fabric and sends them to the relay; refuses plain sending of anything but notional data; writes `/run/simple-bn/status.json` |
| `scene.py` | one shared notional air picture: radars at their sites (CAT034 with position, CAT048 with noise, detection probability and clutter) and a tracker that correlates and filters them into CAT062 system tracks |
| `bn_monitor.py` | read-only status page and `/api/status` for this node's boundary services, mesh address only |
| `sim_status.py` | `simple-bn-sim-status`: declares per feed whether its source is running real or simulated data, when the data does not say; shown at `/api/v1/sim-status`, never applied to what is sent |
| `systemd/` | the two units; the receiver lives in `/etc/simple-bn/receiver.env` (mode 600) |
| encryption stage 2, IPsec | built by the ansible role (`ops/ansible/roles/simple-bn/tasks/ipsec.yml`, swanctl, declaration read by `filter_plugins/simple_bn_ipsec.py`), in policy mode or route mode (an XFRM interface for a route-based gateway, `mode: route` with tunnel addresses), as initiator or as responder for a gateway behind NAT (`initiator: false`, `remote_id` required); the sender's `--encryption ipsec --src-addr <our public or tunnel address>` binds every source port there and fails closed while the tunnel is down; design §4 |
| `tests/` | `python3 -m unittest discover ops/simple-bn/tests` (the fabric end-to-end test runs when `eclipse-zenoh` is installed; the IPsec template tests need `jinja2` and `PyYAML`) |

Try it locally, synthetic data to a local port, monitor on loopback:

```
python3 ops/simple-bn/simple_bn_sender.py --dest 127.0.0.1:10009 --label local --status /tmp/sbn/status.json &
python3 ops/simple-bn/bn_monitor.py --bind 127.0.0.1 --status-dir /tmp/sbn     # http://127.0.0.1:8740/
```

An egress relay that holds, fed by hand on loopback (`sic` checked against each block's first record):

```
echo '{"radar-a": {"title": "Radar A", "sic": 42, "src_port": 40042}}' > /tmp/sbn/p.json
python3 ops/simple-bn/simple_bn_sender.py --input relay --dest none --relay-intake 127.0.0.1 --relay-from 127.0.0.1 \
    --feeds radar-a --profiles /tmp/sbn/p.json --label local --status /tmp/sbn/status.json
```
