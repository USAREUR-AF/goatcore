# simple-bn

A simple boundary node and the boundary monitor. Design and status: [`docs/design/simple-boundary-node.md`](../../docs/design/simple-boundary-node.md); decision [ADR 1253](../../docs/adr/1253-simple-boundary-node-pseudo-boundary-node-and-one-boundary-monitor.md).

| File | What it is |
|---|---|
| `simple_bn_sender.py` | sends notional feeds as ASTERIX CAT034/CAT048 over UDP or TCP (`--transport`) to one receiver, or with `--input relay` forwards a sender's ASTERIX from the mesh unchanged (`--dest none` holds); refuses plain sending of anything but notional data; writes `/run/simple-bn/status.json` |
| `bn_monitor.py` | read-only status page and `/api/status` for this node's boundary services, mesh address only |
| `systemd/` | the two units; the receiver lives in `/etc/simple-bn/receiver.env` (mode 600) |
| `tests/` | `python3 -m unittest discover ops/simple-bn/tests` |

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
