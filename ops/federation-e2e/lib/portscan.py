#!/usr/bin/env python3
"""portscan — a small TCP and UDP port scanner for the node security scenarios.

    python3 - <target> [--tcp 1-65535] [--udp 51820,7448,...] [--timeout 1.0] [--jobs 1500] < portscan.py

Run inside the host that scans (fed via stdin, so nothing is installed there; the host
image carries python3 and no nmap). Prints one JSON object:

  tcp_open      ports that completed a TCP handshake
  tcp_closed    how many ports answered with a reset
  tcp_silent    how many ports gave no answer before the timeout (dropped or unroutable)
  tcp_errors    ports the scanner itself failed to try (with tcp_error, the first reason);
                a scan with any is incomplete and is never judged a pass
  udp           {port: "answered" | "refused" | "silent"}: "refused" when an ICMP port unreachable came
                back, "silent" otherwise. UDP alone cannot tell an open port from a dropped
                one; the unit adds the target's own delivery counters to decide (nodesec.sh).
"""
import argparse
import asyncio
import errno
import json
import resource
import socket
import sys
import time


def ports(spec):
    out = []
    for part in (spec or "").split(","):
        part = part.strip()
        if not part:
            continue
        if "-" in part:
            a, b = part.split("-", 1)
            out.extend(range(int(a), int(b) + 1))
        else:
            out.append(int(part))
    return out


async def tcp_one(host, port, timeout, sem, res):
    async with sem:
        try:
            _, w = await asyncio.wait_for(asyncio.open_connection(host, port), timeout)
            res["open"].append(port)
            w.close()
        except ConnectionRefusedError:
            res["closed"] += 1
        except asyncio.TimeoutError:
            res["silent"] += 1
        except OSError as e:
            if e.errno in (errno.EHOSTUNREACH, errno.ENETUNREACH, errno.ETIMEDOUT):
                res["silent"] += 1
            else:  # the scanner's own failure (out of descriptors, ...): the port is unknown
                res["errors"].append(port)
                res["why"] = repr(e)


async def tcp_scan(host, plist, timeout, jobs):
    res = {"open": [], "closed": 0, "silent": 0, "errors": [], "why": ""}
    sem = asyncio.Semaphore(jobs)
    await asyncio.gather(*(tcp_one(host, p, timeout, sem, res) for p in plist))
    res["open"].sort()
    return res


def udp_scan(host, plist, timeout):
    # Three datagrams per port on a connected socket: a later send or recv raises
    # ECONNREFUSED when the target answered an earlier one with ICMP port unreachable.
    out = {}
    for p in plist:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.settimeout(timeout / 3)
        state = "silent"
        try:
            s.connect((host, p))
            for _ in range(3):
                try:
                    s.send(b"fe2e-portscan\n")
                    s.recv(2048)
                    state = "answered"
                    break
                except socket.timeout:
                    continue
                except ConnectionRefusedError:
                    state = "refused"
                    break
                except OSError as e:
                    if e.errno == errno.ECONNREFUSED:
                        state = "refused"
                        break
                    continue
        finally:
            s.close()
        out[str(p)] = state
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("target")
    ap.add_argument("--tcp", default="1-65535")
    ap.add_argument("--udp", default="")
    ap.add_argument("--timeout", type=float, default=1.0)
    ap.add_argument("--jobs", type=int, default=1500)
    a = ap.parse_args()
    t0 = time.time()
    # One descriptor per port in flight: raise the soft limit to the hard one, and keep the
    # number in flight well under it (a host's default of 1024 silently loses ports).
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    try:
        resource.setrlimit(resource.RLIMIT_NOFILE, (hard, hard))
        soft = hard
    except (ValueError, OSError):
        # The hard limit could not be taken: keep the soft one, and the jobs below fit it.
        pass
    a.jobs = max(16, min(a.jobs, soft - 64))
    tcp = asyncio.run(tcp_scan(a.target, ports(a.tcp), a.timeout, a.jobs))
    udp = udp_scan(a.target, ports(a.udp), max(a.timeout, 1.5))
    json.dump({"target": a.target, "tcp_range": a.tcp, "tcp_open": tcp["open"], "tcp_closed": tcp["closed"],
               "tcp_silent": tcp["silent"], "tcp_errors": tcp["errors"], "tcp_error": tcp["why"], "jobs": a.jobs, "udp": udp, "seconds": round(time.time() - t0, 1)}, sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
