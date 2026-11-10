#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Listen on, or scan, TCP and UDP ports: the helper of netns-behavior.sh.

  probe.py serve tcp:22,7448 udp:51820,5353     answer on every port listed (TCP accepts,
                                                 UDP echoes) until killed
  probe.py scan <src> <dst> tcp:22,7448 udp:51820   print `open` ports, one per line, as
                                                 `tcp/22`; a port is open when the TCP
                                                 connection completes or the UDP echo returns
"""
import selectors
import socket
import sys
import threading


def parse(specs):
    out = []
    for s in specs:
        proto, ports = s.split(":", 1)
        out += [(proto, int(p)) for p in ports.split(",") if p]
    return out


def serve(specs):
    socks = []
    for proto, port in parse(specs):
        if proto == "tcp":
            s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            s.bind(("0.0.0.0", port))
            s.listen(16)
        else:
            s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            s.bind(("0.0.0.0", port))
        socks.append((proto, s))
    sel = selectors.DefaultSelector()
    for proto, s in socks:
        sel.register(s, selectors.EVENT_READ, proto)
    print("ready", flush=True)
    while True:
        for key, _ in sel.select():
            s = key.fileobj
            if key.data == "tcp":
                c, _ = s.accept()
                c.close()
            else:
                data, addr = s.recvfrom(2048)
                s.sendto(data, addr)


def scan(src, dst, specs, timeout=1.0):
    results = {}

    def one(proto, port):
        try:
            if proto == "tcp":
                s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                s.bind((src, 0))
                s.settimeout(timeout)
                s.connect((dst, port))
                results[(proto, port)] = True
            else:
                s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
                s.bind((src, 0))
                s.settimeout(timeout)
                s.sendto(b"probe", (dst, port))
                s.recvfrom(2048)
                results[(proto, port)] = True
            s.close()
        except OSError:
            results[(proto, port)] = False

    threads = [threading.Thread(target=one, args=pp) for pp in parse(specs)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    for (proto, port), ok in sorted(results.items()):
        if ok:
            print(f"{proto}/{port}")


if __name__ == "__main__":
    if sys.argv[1] == "serve":
        serve(sys.argv[2:])
    elif sys.argv[1] == "scan":
        scan(sys.argv[2], sys.argv[3], sys.argv[4:])
    else:
        sys.exit(__doc__)
